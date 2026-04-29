use actix_web::{web, App, HttpResponse, HttpServer};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

#[derive(Deserialize)]
struct QueryParams {
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

/// Full detail for /contracts/{contract_name} endpoint
///
/// From Table "v1.deployed_contracts"
///       Column         |            Type             | Collation | Nullable | Default
///----------------------+-----------------------------+-----------+----------+---------
/// id                   | text                        |           | not null |
/// transaction_hash     | text                        |           |          |
/// ledger_sequence      | bigint                      |           |          |
/// created_at           | timestamp without time zone |           |          |
/// emitter_contract_id  | text                        |           |          |
/// wasm_name            | text                        |           |          |
/// wasm_version         | text                        |           |          |
/// deployer             | text                        |           |          |
/// contract_id          | text                        |           |          |
/// registry_contract_id | text                        |           |          |
///
/// ...and Table "v1.registered_contracts"
///       Column        |            Type             | Collation | Nullable | Default
///---------------------+-----------------------------+-----------+----------+---------
/// id                  | text                        |           | not null |
/// transaction_hash    | text                        |           |          |
/// ledger_sequence     | bigint                      |           |          |
/// created_at          | timestamp without time zone |           |          |
/// emitter_contract_id | text                        |           |          |
/// contract_name       | text                        |           |          |
/// contract_id         | text                        |           |          |
/// sac                 | boolean                     |           |          |
/// initial_wasm_hash   | text                        |           |          |
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
    initial_wasm_hash: Option<String>,
}

/// Row mapping for v1.contract_upgrades. Each row is one host-emitted
/// `executable_update` system event for a contract that's tracked by a
/// registry (root + sub-registries) or registered with one.
#[derive(sqlx::FromRow, Serialize, Clone)]
struct ContractUpgrade {
    id: String,
    transaction_hash: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
    upgraded_contract_id: String,
    old_executable_kind: Option<String>,
    old_wasm_hash: Option<String>,
    new_executable_kind: Option<String>,
    new_wasm_hash: Option<String>,
}

/// Wraps ContractDetail with the wasm-history fields. Flattened so the JSON
/// shape stays a single object.
#[derive(Serialize)]
struct ContractDetailResponse {
    #[serde(flatten)]
    detail: ContractDetail,
    current_wasm_hash: Option<String>,
    wasm_upgrades: Vec<ContractUpgrade>,
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

async fn get_wasms(pool: web::Data<PgPool>, query: web::Query<QueryParams>) -> HttpResponse {
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

    // Groups by wasm_name (priority to the latest publish by ledger_sequence)
    // Edgecase: if there are multiple publishes in the same ledger, rely on semver

    // Finally, all records are sorted first by ledger_sequence (including passed ledger),
    // and then by id (excluding passed id). Because IDs are strings, we transform passed id
    // With adding an extra 'z' symbol to ensure string is lexicographically greater
    // to go to the next transaction in the same ledger (if any)
    let rows = sqlx::query_as::<_, WasmResult>(
        "SELECT sub.id, sub.author, sub.wasm_version, sub.wasm_name, sub.wasm_hash, \
                sub.channel \
         FROM \
           (SELECT *, ROW_NUMBER() OVER \
             (PARTITION BY wasm_name ORDER BY ledger_sequence DESC, wasm_version DESC) AS rn \
             FROM v1.published_wasms_with_channel \
           ) AS sub \
         WHERE rn = 1 AND (ledger_sequence, id) >= ($1, $2) \
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
            eprintln!("Database error: {e}");
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            })
        }
    }
}

async fn fetch_wasm_detail(
    pool: &PgPool,
    channel: &str,
    wasm_name: &str,
    version: Option<&str>,
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
            "SELECT sub.id, sub.transaction_hash, sub.ledger_sequence, sub.created_at, \
                    sub.author, sub.wasm_version, sub.wasm_name, sub.wasm_hash, sub.channel \
             FROM \
               (SELECT *, ROW_NUMBER() OVER \
                 (PARTITION BY wasm_name ORDER BY ledger_sequence DESC, wasm_version DESC) AS rn \
                 FROM v1.published_wasms_with_channel \
               ) AS sub \
             WHERE sub.rn = 1 AND sub.wasm_name = $1 \
               AND sub.channel = $2",
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
                Ok(v) => HttpResponse::Ok().json(WasmDetail {
                    row: detail_row,
                    versions: v,
                }),
                Err(e) => {
                    eprintln!("Database error: {e}");
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: "Internal server error".into(),
                    })
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
            eprintln!("Database error: {e}");
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            })
        }
    }
}

async fn get_wasm_root_channel(pool: web::Data<PgPool>, path: web::Path<String>) -> HttpResponse {
    let wasm_name = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), "root", &wasm_name, None).await
}

async fn get_wasm_latest(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (channel, wasm_name) = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), &channel, &wasm_name, None).await
}

async fn get_wasm_version_root(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (wasm_name, version) = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), "root", &wasm_name, Some(&version)).await
}

async fn get_wasm_version(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String, String)>,
) -> HttpResponse {
    let (channel, wasm_name, version) = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), &channel, &wasm_name, Some(&version)).await
}

async fn get_contracts_root(
    pool: web::Data<PgPool>,
    query: web::Query<QueryParams>,
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
        "SELECT
                registered.id,
                registered.contract_id,
                registered.channel,
                registered.contract_name,
                registered.sac,
                deployed.deployer,
                wasms.wasm_version,
                wasms.wasm_name,
                registries.registry_channel AS wasm_channel
            FROM v1.registered_contracts_with_channel registered
            LEFT JOIN (
                SELECT DISTINCT ON (wasm_hash) wasm_hash, wasm_version, wasm_name
                FROM v1.published_wasms
                ORDER BY wasm_hash, ledger_sequence DESC
            ) wasms ON wasms.wasm_hash = registered.initial_wasm_hash
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, deployer, registry_contract_id
                FROM v1.deployed_contracts
                ORDER BY contract_id, ledger_sequence DESC
            ) deployed ON deployed.contract_id = registered.contract_id
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, registry_channel
                FROM v1.registries
            ) registries ON deployed.registry_contract_id = registries.contract_id
            WHERE (registered.ledger_sequence, registered.id) >= ($1, $2)
            ORDER BY registered.ledger_sequence, registered.id ASC
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
            eprintln!("Database error: {e}");
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            })
        }
    }
}

async fn get_single_contract_root(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    let contract_name = path.into_inner();

    fetch_single_contract("root", &contract_name, pool).await
}

async fn get_single_contract(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (channel, contract_name) = path.into_inner();
    fetch_single_contract(&channel, &contract_name, pool).await
}

async fn fetch_single_contract(
    channel: &str,
    contract_name: &str,
    pool: web::Data<PgPool>,
) -> HttpResponse {
    let row = sqlx::query_as::<_, ContractDetail>(
        "SELECT
                registered.id,
                registered.transaction_hash,
                registered.ledger_sequence,
                registered.created_at,
                registered.contract_id,
                registered.contract_name,
                registered.channel,
                registered.sac,
                registered.initial_wasm_hash,
                deployed.deployer,
                wasms.wasm_version,
                wasms.wasm_name,
                registries.registry_channel AS wasm_channel
            FROM v1.registered_contracts_with_channel registered
            LEFT JOIN (
                SELECT DISTINCT ON (wasm_hash) wasm_hash, wasm_version, wasm_name
                FROM v1.published_wasms
                ORDER BY wasm_hash, ledger_sequence DESC
            ) wasms ON wasms.wasm_hash = registered.initial_wasm_hash
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, deployer, registry_contract_id
                FROM v1.deployed_contracts
                ORDER BY contract_id, ledger_sequence DESC
            ) deployed ON deployed.contract_id = registered.contract_id
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, registry_channel
                FROM v1.registries
            ) registries ON deployed.registry_contract_id = registries.contract_id
            WHERE registered.contract_name = $1
              AND registered.channel = $2
            ORDER BY registered.ledger_sequence DESC
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
            eprintln!("Database error: {e}");
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            });
        }
    };

    let Some(contract_id) = detail.contract_id.clone() else {
        return HttpResponse::Ok().json(ContractDetailResponse {
            current_wasm_hash: detail.initial_wasm_hash.clone(),
            wasm_upgrades: vec![],
            detail,
        });
    };

    let upgrades = match fetch_upgrades_for_contract_id(&contract_id, pool.get_ref()).await {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("Database error: {e}");
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            });
        }
    };

    let current_wasm_hash = upgrades
        .last()
        .and_then(|u| u.new_wasm_hash.clone())
        .or_else(|| detail.initial_wasm_hash.clone());

    HttpResponse::Ok().json(ContractDetailResponse {
        detail,
        current_wasm_hash,
        wasm_upgrades: upgrades,
    })
}

async fn fetch_upgrades_for_contract_id(
    contract_id: &str,
    pool: &PgPool,
) -> Result<Vec<ContractUpgrade>, sqlx::Error> {
    sqlx::query_as::<_, ContractUpgrade>(
        "SELECT
                id,
                transaction_hash,
                ledger_sequence,
                created_at,
                upgraded_contract_id,
                old_executable_kind,
                old_wasm_hash,
                new_executable_kind,
                new_wasm_hash
            FROM v1.contract_upgrades
            WHERE upgraded_contract_id = $1
            ORDER BY ledger_sequence ASC",
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await
}

async fn get_contract_deploy_detail(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (channel, contract_name) = path.into_inner();
    fetch_single_contract_detail(&channel, &contract_name, pool).await
}

async fn fetch_single_contract_detail(
    channel: &str,
    contract_name: &str,
    pool: web::Data<PgPool>,
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
            eprintln!("Database error: {e}");
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            })
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
            { "method": "GET", "path": "/v1/contracts/{contract_name}", "description": "Get details for a deployed contract (main channel), including current_wasm_hash and wasm_upgrades history" },
            { "method": "GET", "path": "/v1/contracts/{channel}/{contract_name}", "description": "Get details for a deployed contract for a specific channel, including current_wasm_hash and wasm_upgrades history" },
            { "method": "GET", "path": "/v1/registries", "description": "List all known sub-registries announced by the root registry." },
        ]
    }))
}

async fn get_registries(pool: web::Data<PgPool>) -> HttpResponse {
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
            eprintln!("Database error: {e}");
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Internal server error".into(),
            })
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

    println!("Starting server on port {port}");

    HttpServer::new(move || {
        App::new()
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
