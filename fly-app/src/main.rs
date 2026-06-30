use actix_web::{web, App, HttpResponse, HttpServer};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use stellar_xdr::curr::{ScMetaEntry, ScMetaV0};
use tracing_actix_web::{DefaultRootSpanBuilder, RequestId, TracingLogger};

use crate::tracing::init_tracing;
mod tracing;

#[derive(Deserialize)]
struct QueryParams {
    query: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

/// Slim result for /wasms list endpoint
#[derive(sqlx::FromRow, Serialize)]
struct WasmResult {
    #[serde(skip)]
    id: String,
    author: Option<String>,
    channel: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
    wasm_hash: Option<String>,
}

/// Slim result for versions array
#[derive(sqlx::FromRow, Serialize)]
struct WasmVersionResult {
    author: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
    wasm_hash: Option<String>,
}

/// DB row mapping for v1.published_wasms
///
/// ```
/// Column      |            Type             | Collation | Nullable | Default
/// ------------------+-----------------------------+-----------+----------+---------
/// id               | text                        |           | not null |
/// transaction_hash | text                        |           | not null |
/// ledger_sequence  | bigint                      |           | not null |
/// created_at       | timestamp without time zone |           | not null |
/// channel          | text                        |           |          |
/// author           | text                        |           |          |
/// wasm_version     | text                        |           |          |
/// wasm_hash        | text                        |           |          |
/// wasm_name        | text                        |           |          |
/// ```
#[derive(sqlx::FromRow, Serialize)]
struct WasmDetailRow {
    id: String,
    transaction_hash: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
    channel: Option<String>,
    author: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
    wasm_hash: Option<String>,
}

/// Full detail for /wasms/{wasm_name} endpoint
#[derive(Serialize)]
struct WasmDetail {
    #[serde(flatten)]
    row: WasmDetailRow,
    versions: Vec<WasmVersionResult>,
    meta: Option<WasmMeta>,
}

#[derive(Serialize, Deserialize)]
struct WasmMeta {
    rsver: Option<String>,
    rssdkver: Option<String>,
    rssdk_spec_shaking: Option<String>,
    cliver: Option<String>,
    source_repo: Option<String>,
    binver: Option<String>,
}

/// Slim result for /contracts list endpoint
#[derive(sqlx::FromRow, Serialize)]
struct ContractResult {
    #[serde(skip)]
    id: String,
    channel: Option<String>,
    contract_id: Option<String>,
    contract_name: Option<String>,
    deployer: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
    wasm_channel: Option<String>,
    #[serde(rename = "is_stellar_asset_contract")]
    sac: Option<bool>,
}

/// Full detail for /contracts/{contract_name} endpoint, surfaced via the
/// contracts_enriched view (registered contracts decorated with deployer
/// + wasm publish metadata + wasm_channel). The contract's wasm history
/// is returned separately in ContractDetailResponse.versions.
#[derive(sqlx::FromRow, Serialize)]
struct ContractDetail {
    id: String,
    transaction_hash: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
    contract_id: Option<String>,
    contract_name: Option<String>,
    channel: Option<String>,
    deployer: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
    wasm_channel: Option<String>,
    #[serde(rename = "is_stellar_asset_contract")]
    sac: Option<bool>,
}

/// Row mapping for v1.versions — one row per (contract × wasm transition),
/// chronologically ordered within a contract. kind is 'initial' for the
/// deploy row, 'upgrade' for runtime executable_update events. wasm_name,
/// wasm_version, and wasm_channel come from a join against published_wasms
/// and the originating registry; all three are NULL for wasms that were
/// uploaded but never published.
#[derive(sqlx::FromRow, Serialize, Clone)]
struct ContractVersion {
    version_index: i64,
    kind: String,
    wasm_hash: Option<String>,
    wasm_name: Option<String>,
    wasm_version: Option<String>,
    wasm_channel: Option<String>,
    transaction_hash: Option<String>,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
}

/// Wraps ContractDetail with the contract's wasm version history.
/// Flattened so the JSON shape stays a single object.
#[derive(Serialize)]
struct ContractDetailResponse {
    #[serde(flatten)]
    detail: ContractDetail,
    versions: Vec<ContractVersion>,
}

/// From Table "v1.registries"
///      Column      |            Type             | Collation | Nullable | Default
///------------------+-----------------------------+-----------+----------+---------
/// id               | text                        |           |          |
/// transaction_hash | text                        |           |          |
/// ledger_sequence  | bigint                      |           |          |
/// created_at       | timestamp without time zone |           |          |
/// contract_id      | text                        |           | not null |
/// registry_channel | text                        |           |          |
#[derive(sqlx::FromRow, Serialize)]
struct Registry {
    contract_id: String,
    channel: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
}

#[derive(sqlx::FromRow, Serialize)]
struct ContractDeployDetail {
    contract_id: Option<String>,
    contract_name: Option<String>,
    channel: Option<String>,
    deployer: Option<String>,
    #[serde(serialize_with = "serialize_raw")]
    operation_body: Option<String>,
}

pub fn serialize_raw<S: serde::Serializer>(val: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    match val {
        None => s.serialize_none(),
        Some(raw) => {
            let v: serde_json::Value =
                serde_json::from_str(raw).map_err(serde::ser::Error::custom)?;
            v.serialize(s)
        }
    }
}

#[derive(Serialize)]
struct ListResponse<T: Serialize> {
    result: Vec<T>,
    next: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize)]
struct InternalErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
}

fn internal_server_error_response(request_id: RequestId) -> HttpResponse {
    HttpResponse::InternalServerError().json(InternalErrorResponse {
        error: "Internal server error".into(),
        request_id: Some(request_id.to_string()),
    })
}

fn log_db_error(operation: &'static str, error: &sqlx::Error, pool: &PgPool) {
    ::tracing::error!(
        operation,
        error = %error,
        error_debug = ?error,
        pool_size = pool.size(),
        pool_idle = pool.num_idle(),
        "database query failed"
    );
}

async fn get_wasms(
    pool: web::Data<PgPool>,
    query: web::Query<QueryParams>,
    request_id: RequestId,
) -> HttpResponse {
    let limit = query.limit.unwrap_or(200);
    if limit < 1 || limit > 200 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 1 and 200".into(),
        });
    }

    let (ledger, cursor) = match parse_cursor(&query.cursor) {
        Ok(val) => val,
        Err(resp) => return resp,
    };

    let rows = sqlx::query_as::<_, WasmResult>(
        "SELECT id, author, wasm_version, wasm_name, wasm_hash, channel, \
                CASE \
                    WHEN $4::text IS NULL THEN 0 \
                    ELSE GREATEST( \
                        v1.similarity(wasm_name, $4), \
                        v1.similarity(channel, $4), \
                        CASE WHEN wasm_hash = $4 THEN 1.0 ELSE 0.0 END, \
                        CASE WHEN author = $4 THEN 1.0 ELSE 0.0 END \
                    ) \
                END AS rank \
         FROM v1.latest_published_wasms \
         WHERE (ledger_sequence, id) >= ($1, $2) \
            AND (
                $4::text IS NULL \
                OR (v1.similarity(wasm_name, $4) > 0.2 \
                OR wasm_hash = $4 \
                OR author = $4 \
                OR v1.similarity(channel, $4) > 0.2 ) \
                ) \
         ORDER BY rank DESC, ledger_sequence, id ASC \
         LIMIT $3",
    )
    .bind(ledger)
    .bind(&cursor)
    .bind(limit)
    .bind(query.query.as_deref())
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(rows) => {
            let next = if rows.len() as i64 == limit {
                rows.last().map(|r| r.id.clone())
            } else {
                None
            };
            HttpResponse::Ok().json(ListResponse { result: rows, next })
        }
        Err(e) => {
            log_db_error("get_wasms.fetch_latest_published_wasms", &e, pool.get_ref());
            internal_server_error_response(request_id)
        }
    }
}

async fn fetch_wasm_meta(pool: &PgPool, wasm_hash: &str) -> Option<WasmMeta> {
    let wasm_bytes = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT decode(wasm, 'hex') FROM archive.wasm_binaries WHERE wasm_hash = $1",
    )
    .bind(wasm_hash)
    .fetch_optional(pool)
    .await;

    match wasm_bytes {
        Ok(Some(bytes)) => {
            let meta = soroban_meta::read::from_wasm(&bytes);
            match meta {
                Ok(entries) => {
                    let mut obj = serde_json::Map::new();
                    for entry in entries {
                        let ScMetaEntry::ScMetaV0(ScMetaV0 { key, val }) = entry;
                        obj.insert(
                            key.to_utf8_string_lossy(),
                            serde_json::Value::String(val.to_utf8_string_lossy()),
                        );
                    }
                    let wasm_meta =
                        match serde_json::from_value::<WasmMeta>(serde_json::Value::Object(obj)) {
                            Ok(m) => m,
                            Err(e) => {
                                ::tracing::warn!(
                                    wasm_hash,
                                    error = %e,
                                    error_debug = ?e,
                                    "failed to deserialize wasm metadata"
                                );
                                return None;
                            }
                        };
                    return Some(wasm_meta);
                }
                Err(e) => {
                    ::tracing::warn!(
                        wasm_hash,
                        error = %e,
                        error_debug = ?e,
                        "failed to read wasm metadata"
                    );
                    return None;
                }
            }
        }
        Ok(None) => {
            ::tracing::warn!(wasm_hash, "no wasm binary found");
            return None;
        }
        Err(e) => {
            log_db_error("fetch_wasm_meta.select_wasm_binary", &e, pool);
            return None;
        }
    }
}

async fn fetch_wasm_detail(
    pool: &PgPool,
    channel: &str,
    wasm_name: &str,
    version: Option<&str>,
    request_id: RequestId,
) -> HttpResponse {
    let row = if let Some(ver) = version {
        sqlx::query_as::<_, WasmDetailRow>(
            "SELECT id, transaction_hash, ledger_sequence, created_at, \
                    author, wasm_version, wasm_name, wasm_hash, channel \
             FROM v1.published_wasms_with_channel \
             WHERE wasm_name = $1 AND wasm_version = $2 \
               AND channel = $3",
        )
        .bind(wasm_name)
        .bind(ver)
        .bind(channel)
        .fetch_optional(pool)
        .await
    } else {
        sqlx::query_as::<_, WasmDetailRow>(
            "SELECT id, transaction_hash, ledger_sequence, created_at, \
                    author, wasm_version, wasm_name, wasm_hash, channel \
             FROM v1.latest_published_wasms \
             WHERE wasm_name = $1 AND channel = $2",
        )
        .bind(wasm_name)
        .bind(channel)
        .fetch_optional(pool)
        .await
    };

    match row {
        // TODO: can do only one select and filter the results
        Ok(Some(detail_row)) => {
            let versions = sqlx::query_as::<_, WasmVersionResult>(
                "SELECT author, wasm_version, wasm_name, wasm_hash, channel \
                 FROM v1.published_wasms_with_channel \
                 WHERE wasm_name = $1 \
                   AND channel = $2 \
                 ORDER BY ledger_sequence DESC, wasm_version DESC",
            )
            .bind(wasm_name)
            .bind(channel)
            .fetch_all(pool)
            .await;

            match versions {
                Ok(v) => {
                    let wasm_meta = if let Some(wasm_hash) = detail_row.wasm_hash.as_deref() {
                        fetch_wasm_meta(pool, wasm_hash).await
                    } else {
                        ::tracing::warn!(
                            wasm_name,
                            channel,
                            version = ?version,
                            "missing wasm_hash; returning wasm detail without metadata"
                        );
                        None
                    };
                    HttpResponse::Ok().json(WasmDetail {
                        row: detail_row,
                        versions: v,
                        meta: wasm_meta,
                    })
                }
                Err(e) => {
                    log_db_error("fetch_wasm_detail.select_wasm_versions", &e, pool);
                    internal_server_error_response(request_id)
                }
            }
        }
        Ok(None) => {
            let msg = if let Some(ver) = version {
                format!("Wasm '{wasm_name}' version '{ver}' not found")
            } else {
                format!("Wasm '{wasm_name}' not found")
            };
            HttpResponse::NotFound().json(ErrorResponse { error: msg })
        }
        Err(e) => {
            log_db_error("fetch_wasm_detail.select_wasm_detail", &e, pool);
            internal_server_error_response(request_id)
        }
    }
}

async fn get_wasm_root_channel(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    request_id: RequestId,
) -> HttpResponse {
    let wasm_name = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), "root", &wasm_name, None, request_id).await
}

async fn get_wasm_latest(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
    request_id: RequestId,
) -> HttpResponse {
    let (channel, wasm_name) = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), &channel, &wasm_name, None, request_id).await
}

async fn get_wasm_version_root(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
    request_id: RequestId,
) -> HttpResponse {
    let (wasm_name, version) = path.into_inner();
    fetch_wasm_detail(
        pool.get_ref(),
        "root",
        &wasm_name,
        Some(&version),
        request_id,
    )
    .await
}

async fn get_wasm_version(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String, String)>,
    request_id: RequestId,
) -> HttpResponse {
    let (channel, wasm_name, version) = path.into_inner();
    fetch_wasm_detail(
        pool.get_ref(),
        &channel,
        &wasm_name,
        Some(&version),
        request_id,
    )
    .await
}

async fn get_contracts_root(
    pool: web::Data<PgPool>,
    query: web::Query<QueryParams>,
    request_id: RequestId,
) -> HttpResponse {
    let limit = query.limit.unwrap_or(200);
    if limit < 1 || limit > 200 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 1 and 200".into(),
        });
    }

    let (ledger, cursor) = match parse_cursor(&query.cursor) {
        Ok(val) => val,
        Err(resp) => return resp,
    };

    let rows = sqlx::query_as::<_, ContractResult>(
        "SELECT id, contract_id, channel, contract_name, sac, deployer, \
                wasm_version, wasm_name, wasm_channel \
         FROM v1.contracts_enriched \
         WHERE (ledger_sequence, id) >= ($1, $2) \
         ORDER BY ledger_sequence, id ASC \
         LIMIT $3",
    )
    .bind(ledger)
    .bind(&cursor)
    .bind(limit)
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(rows) => {
            let next = if rows.len() as i64 == limit {
                rows.last().map(|r| r.id.clone())
            } else {
                None
            };

            HttpResponse::Ok().json(ListResponse { result: rows, next })
        }
        Err(e) => {
            log_db_error("get_contracts_root.fetch_contracts", &e, pool.get_ref());
            internal_server_error_response(request_id)
        }
    }
}

async fn get_single_contract_root(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    request_id: RequestId,
) -> HttpResponse {
    let contract_name = path.into_inner();
    fetch_single_contract("root", &contract_name, pool, request_id).await
}

async fn get_single_contract(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
    request_id: RequestId,
) -> HttpResponse {
    let (channel, contract_name) = path.into_inner();
    fetch_single_contract(&channel, &contract_name, pool, request_id).await
}

async fn fetch_single_contract(
    channel: &str,
    contract_name: &str,
    pool: web::Data<PgPool>,
    request_id: RequestId,
) -> HttpResponse {
    let row = sqlx::query_as::<_, ContractDetail>(
        "SELECT id, transaction_hash, ledger_sequence, created_at, \
                contract_id, contract_name, channel, sac, \
                deployer, wasm_version, wasm_name, wasm_channel \
         FROM v1.contracts_enriched \
         WHERE contract_name = $1 AND channel = $2 \
         ORDER BY ledger_sequence DESC \
         LIMIT 1",
    )
    .bind(&contract_name)
    .bind(&channel)
    .fetch_optional(pool.get_ref())
    .await;

    let detail = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return HttpResponse::NotFound().json(ErrorResponse {
                error: format!("Contract '{contract_name}' not found"),
            });
        }
        Err(e) => {
            log_db_error(
                "fetch_single_contract.select_contract_detail",
                &e,
                pool.get_ref(),
            );
            return internal_server_error_response(request_id);
        }
    };

    let Some(contract_id) = detail.contract_id.clone() else {
        return HttpResponse::Ok().json(ContractDetailResponse {
            versions: vec![],
            detail,
        });
    };

    let versions = match fetch_versions_for_contract_id(&contract_id, pool.get_ref()).await {
        Ok(rows) => rows,
        Err(e) => {
            log_db_error(
                "fetch_single_contract.select_contract_versions",
                &e,
                pool.get_ref(),
            );
            return internal_server_error_response(request_id);
        }
    };

    HttpResponse::Ok().json(ContractDetailResponse { detail, versions })
}

async fn fetch_versions_for_contract_id(
    contract_id: &str,
    pool: &PgPool,
) -> Result<Vec<ContractVersion>, sqlx::Error> {
    sqlx::query_as::<_, ContractVersion>(
        "SELECT version_index, kind, wasm_hash, wasm_name, wasm_version, wasm_channel, \
                transaction_hash, ledger_sequence, created_at \
         FROM v1.versions \
         WHERE contract_id = $1 \
         ORDER BY version_index ASC",
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await
}

async fn get_contract_deploy_detail(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
    request_id: RequestId,
) -> HttpResponse {
    let (channel, contract_name) = path.into_inner();
    fetch_single_contract_detail(&channel, &contract_name, pool, request_id).await
}

async fn fetch_single_contract_detail(
    channel: &str,
    contract_name: &str,
    pool: web::Data<PgPool>,
    request_id: RequestId,
) -> HttpResponse {
    let row = sqlx::query_as::<_, ContractDeployDetail>(
        "SELECT
                registered.contract_id,
                registered.contract_name,
                registered.channel,
                deployed.deployer,
                raw_event.operation_body
            FROM v1.registered_contracts_with_channel registered
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, deployer, transaction_hash
                FROM v1.deployed_contracts
                ORDER BY contract_id, ledger_sequence DESC
            ) deployed ON deployed.contract_id = registered.contract_id
            LEFT JOIN v1.raw_events_backup raw_event
              ON deployed.transaction_hash = raw_event.contract_id
            WHERE registered.contract_name = $1
              AND registered.channel = $2
            ORDER BY registered.ledger_sequence DESC
            LIMIT 1",
    )
    .bind(&contract_name)
    .bind(&channel)
    .fetch_optional(pool.get_ref())
    .await;

    match row {
        Ok(Some(r)) => {
            if r.operation_body.is_some() {
                HttpResponse::Ok().json(r)
            } else {
                HttpResponse::NotFound().json(ErrorResponse {
                    error: format!("Contract '{contract_name}' deploy details are not found"),
                })
            }
        }
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Contract '{contract_name}' not found"),
        }),
        Err(e) => {
            log_db_error(
                "fetch_single_contract_detail.select_contract_deploy_detail",
                &e,
                pool.get_ref(),
            );
            internal_server_error_response(request_id)
        }
    }
}

fn parse_cursor(cursor: &Option<String>) -> Result<(i64, String), HttpResponse> {
    let Some(cursor) = cursor else {
        return Ok((0, String::new()));
    };

    let parts: Vec<&str> = cursor.splitn(3, '-').collect();
    if parts.len() < 2 {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid cursor".into(),
        }));
    }

    let ledger: i64 = parts[0].parse().map_err(|_| {
        HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid cursor".into(),
        })
    })?;

    if ledger < 0 {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid cursor".into(),
        }));
    }

    // `id` format is <ledger>-<tx hash>-op-<op number>-event-<event number>
    // Append 'z' to make the cursor lexicographically greater, advancing past
    // the current transaction within the same ledger.
    let cursor = format!("{}-z", cursor);
    Ok((ledger, cursor))
}

async fn index() -> HttpResponse {
    // Version status: current | deprecated | sunset
    HttpResponse::Ok().json(serde_json::json!({
        "name": "Registry Indexer API",
        "versions": [
            { "version": "v1", "path": "/v1", "status": "current" }
        ]
    }))
}

async fn index_v1() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "name": "Registry Indexer API v1",
        "endpoints": [
            { "method": "GET", "path": "/v1/wasms", "description": "List all published wasms (latest version per name, main channel)" },
            { "method": "GET", "path": "/v1/wasms/{wasm_name}", "description": "Get the latest version of a wasm (main channel)" },
            { "method": "GET", "path": "/v1/wasms/{channel}/{wasm_name}", "description": "Get the latest version of a wasm for a specific channel. Supported channels: main, unverified" },
            { "method": "GET", "path": "/v1/wasms/{wasm_name}/v/{version}", "description": "Get a specific version of a wasm (main channel)" },
            { "method": "GET", "path": "/v1/wasms/{channel}/{wasm_name}/v/{version}", "description": "Get a specific version of a wasm for a specific channel. Supported channels: main, unverified" },
            { "method": "GET", "path": "/v1/contracts", "description": "List all deployed contracts (main channel)" },
            { "method": "GET", "path": "/v1/contracts/{contract_name}", "description": "Get details for a deployed contract (main channel), including the wasm versions history" },
            { "method": "GET", "path": "/v1/contracts/{channel}/{contract_name}", "description": "Get details for a deployed contract for a specific channel, including the wasm versions history" },
            { "method": "GET", "path": "/v1/registries", "description": "List all known sub-registries announced by the root registry." },
        ]
    }))
}

async fn get_registries(pool: web::Data<PgPool>, request_id: RequestId) -> HttpResponse {
    let rows = sqlx::query_as::<_, Registry>(
        "SELECT contract_id, registry_channel as channel, ledger_sequence, created_at \
         FROM v1.registries \
         ORDER BY channel ASC",
    )
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(rows) => HttpResponse::Ok().json(ListResponse::<Registry> {
            result: rows,
            next: None,
        }),
        Err(e) => {
            log_db_error("get_registries.fetch_registries", &e, pool.get_ref());
            internal_server_error_response(request_id)
        }
    }
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().finish()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .expect("PORT must be a valid number");

    init_tracing();

    ::tracing::info!(
        port,
        pool_size = pool.size(),
        pool_idle = pool.num_idle(),
        "starting server"
    );

    HttpServer::new(move || {
        let tracing_middleware = TracingLogger::<DefaultRootSpanBuilder>::new();
        App::new()
            .wrap(tracing_middleware)
            .app_data(web::Data::new(pool.clone()))
            .route("/", web::get().to(index))
            .route("/v1", web::get().to(index_v1))
            .route("/v1/wasms", web::get().to(get_wasms))
            .route(
                "/v1/wasms/{wasm_name}",
                web::get().to(get_wasm_root_channel),
            )
            .route(
                "/v1/wasms/{channel}/{wasm_name}",
                web::get().to(get_wasm_latest),
            )
            .route(
                "/v1/wasms/{wasm_name}/v/{version}",
                web::get().to(get_wasm_version_root),
            )
            .route(
                "/v1/wasms/{channel}/{wasm_name}/v/{version}",
                web::get().to(get_wasm_version),
            )
            .route("/v1/registries", web::get().to(get_registries))
            .route("/v1/contracts", web::get().to(get_contracts_root))
            .route(
                "/v1/contract_deploy_details/{channel}/{contract_name}",
                web::get().to(get_contract_deploy_detail),
            )
            .route(
                "/v1/contracts/{contract_name}",
                web::get().to(get_single_contract_root),
            )
            .route(
                "/v1/contracts/{channel}/{contract_name}",
                web::get().to(get_single_contract),
            )
            .route("/health", web::get().to(health))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
