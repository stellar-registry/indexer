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

/// DB row mapping for v4_published_wasms
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
/// From Table "public.v4_deployed_contracts"
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
/// ...and Table "public.v4_registered_contracts"
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
/// wasm_hash           | text                        |           |          |
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

/// From Table "public.v4_registries"
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
             FROM public.v4_published_wasms_with_channel \
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
             FROM public.v4_published_wasms_with_channel \
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
                 FROM public.v4_published_wasms_with_channel \
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
                 FROM public.v4_published_wasms_with_channel \
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
            FROM public.v4_registered_contracts_with_channel registered
            LEFT JOIN (
                SELECT DISTINCT ON (wasm_hash) wasm_hash, wasm_version, wasm_name
                FROM public.v4_published_wasms
                ORDER BY wasm_hash, ledger_sequence DESC
            ) wasms ON wasms.wasm_hash = registered.wasm_hash
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, deployer, registry_contract_id
                FROM public.v4_deployed_contracts
                ORDER BY contract_id, ledger_sequence DESC
            ) deployed ON deployed.contract_id = registered.contract_id
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, registry_channel
                FROM public.v4_registries
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
                deployed.deployer,
                wasms.wasm_version,
                wasms.wasm_name,
                registries.registry_channel AS wasm_channel
            FROM public.v4_registered_contracts_with_channel registered
            LEFT JOIN (
                SELECT DISTINCT ON (wasm_hash) wasm_hash, wasm_version, wasm_name
                FROM public.v4_published_wasms
                ORDER BY wasm_hash, ledger_sequence DESC
            ) wasms ON wasms.wasm_hash = registered.wasm_hash
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, deployer, registry_contract_id
                FROM public.v4_deployed_contracts
                ORDER BY contract_id, ledger_sequence DESC
            ) deployed ON deployed.contract_id = registered.contract_id
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, registry_channel
                FROM public.v4_registries
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

    match row {
        Ok(Some(r)) => HttpResponse::Ok().json(r),
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
            FROM public.v4_registered_contracts_with_channel registered
            LEFT JOIN (
                SELECT DISTINCT ON (contract_id) contract_id, deployer, transaction_hash
                FROM public.v4_deployed_contracts
                ORDER BY contract_id, ledger_sequence DESC
            ) deployed ON deployed.contract_id = registered.contract_id
            LEFT JOIN public.v4_raw_events_backup raw_event
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
            { "method": "GET", "path": "/v1/contracts/{contract_name}", "description": "Get details for a deployed contract (main channel)" },
            { "method": "GET", "path": "/v1/contracts/{channel}/{contract_name}", "description": "Get details for a deployed contract for a specific channel." },
            { "method": "GET", "path": "/v1/registries", "description": "List all known sub-registries announced by the root registry." },
        ]
    }))
}

async fn get_registries(pool: web::Data<PgPool>) -> HttpResponse {
    let rows = sqlx::query_as::<_, Registry>(
        "SELECT contract_id, registry_channel as channel, ledger_sequence, created_at \
         FROM public.v4_registries \
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

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(index))
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
            "/v1/contracts/{contract_name}",
            web::get().to(get_single_contract_root),
        )
        .route(
            "/v1/contracts/{channel}/{contract_name}",
            web::get().to(get_single_contract),
        )
        .route(
            "/v1/contract_deploy_details/{channel}/{contract_name}",
            web::get().to(get_contract_deploy_detail),
        )
        .route("/health", web::get().to(health));
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
            .configure(configure_routes)
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helpers shared by the DB-backed submodules (`endpoints` and
    // `e2e_real_data`). `make_app!` is a macro (not a fn) because the
    // concrete `impl Service` type returned by `test::init_service` is hard
    // to name in a helper signature; expanding at the call site sidesteps it.
    async fn body_json<B: actix_web::body::MessageBody>(
        resp: actix_web::dev::ServiceResponse<B>,
    ) -> serde_json::Value {
        actix_web::test::read_body_json(resp).await
    }

    macro_rules! make_app {
        ($pool:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(actix_web::web::Data::new($pool))
                    .configure(configure_routes),
            )
            .await
        };
    }

    mod parse_cursor_tests {
        use super::*;

        #[test]
        fn none_returns_zero_and_empty() {
            let (ledger, cursor) = parse_cursor(&None).unwrap();
            assert_eq!(ledger, 0);
            assert_eq!(cursor, "");
        }

        #[test]
        fn valid_two_segment_appends_z() {
            let (ledger, cursor) = parse_cursor(&Some("12345-abcdef".into())).unwrap();
            assert_eq!(ledger, 12345);
            assert_eq!(cursor, "12345-abcdef-z");
        }

        #[test]
        fn valid_three_segment_appends_z() {
            // splitn(3, '-') keeps everything past the second '-' in one piece.
            let (ledger, cursor) =
                parse_cursor(&Some("99-hash-op-0-event-1".into())).unwrap();
            assert_eq!(ledger, 99);
            assert_eq!(cursor, "99-hash-op-0-event-1-z");
        }

        #[test]
        fn single_segment_is_rejected() {
            assert!(parse_cursor(&Some("12345".into())).is_err());
        }

        #[test]
        fn non_numeric_ledger_is_rejected() {
            assert!(parse_cursor(&Some("abc-def".into())).is_err());
        }

        #[test]
        fn negative_ledger_is_rejected() {
            assert!(parse_cursor(&Some("-1-foo".into())).is_err());
        }
    }

    mod serialize_raw_tests {
        use super::*;
        use serde::Serialize;

        #[derive(Serialize)]
        struct Wrap(#[serde(serialize_with = "serialize_raw")] Option<String>);

        #[test]
        fn none_serializes_to_null() {
            assert_eq!(serde_json::to_string(&Wrap(None)).unwrap(), "null");
        }

        #[test]
        fn valid_json_string_is_inlined() {
            let w = Wrap(Some(r#"{"foo":42}"#.into()));
            // Round-trip via serde_json::Value so we compare semantically,
            // not by key-ordering of serialize output.
            let actual: serde_json::Value = serde_json::from_str(
                &serde_json::to_string(&w).unwrap(),
            )
            .unwrap();
            assert_eq!(actual, serde_json::json!({"foo": 42}));
        }

        #[test]
        fn invalid_json_produces_error() {
            let w = Wrap(Some("not valid json".into()));
            assert!(serde_json::to_string(&w).is_err());
        }
    }

    // Integration tests below hit a real Postgres. They're gated on
    // TEST_DATABASE_URL; when unset, each test logs a skip message and
    // returns. CI sets the env var to the service container URL.
    mod endpoints {
        use super::*;
        use actix_web::{http::StatusCode, test};
        use serial_test::serial;
        use sqlx::postgres::PgPoolOptions;
        use sqlx::Executor;

        async fn setup_pool() -> Option<PgPool> {
            let url = std::env::var("TEST_DATABASE_URL").ok()?;
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .expect("connect to test db");
            apply_schema(&pool).await;
            truncate_all(&pool).await;
            Some(pool)
        }

        async fn apply_schema(pool: &PgPool) {
            let manifest = env!("CARGO_MANIFEST_DIR");
            for rel in [
                "../sql/v4_sink_tables.sql",
                "../sql/v4_registries.sql",
                "../sql/v4_named_views.sql",
            ] {
                let path = format!("{manifest}/{rel}");
                let sql = tokio::fs::read_to_string(&path)
                    .await
                    .unwrap_or_else(|e| panic!("read {path}: {e}"));
                sqlx::raw_sql(&sql)
                    .execute(pool)
                    .await
                    .unwrap_or_else(|e| panic!("apply {path}: {e}"));
            }
        }

        async fn truncate_all(pool: &PgPool) {
            pool.execute(
                "TRUNCATE v4_published_wasms, v4_registered_contracts, \
                 v4_deployed_contracts, v4_registries, v4_rename, \
                 v4_update_address, v4_update_owner, v4_raw_events_backup \
                 RESTART IDENTITY",
            )
            .await
            .expect("truncate");
        }

        fn ts(s: &str) -> chrono::NaiveDateTime {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
        }

        async fn insert_wasm(
            pool: &PgPool,
            id: &str,
            ledger: i64,
            channel: &str,
            name: &str,
            version: &str,
            hash: &str,
        ) {
            sqlx::query(
                "INSERT INTO v4_published_wasms \
                   (id, transaction_hash, ledger_sequence, created_at, \
                    channel, author, wasm_version, wasm_name, wasm_hash) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(id)
            .bind(format!("tx-{id}"))
            .bind(ledger)
            .bind(ts("2026-04-17 12:00:00"))
            .bind(channel)
            .bind("GAUTHOR")
            .bind(version)
            .bind(name)
            .bind(hash)
            .execute(pool)
            .await
            .expect("insert wasm");
        }

        #[allow(clippy::too_many_arguments)]
        async fn insert_registered(
            pool: &PgPool,
            id: &str,
            ledger: i64,
            channel: &str,
            contract_name: &str,
            contract_id: &str,
            wasm_hash: &str,
            sac: bool,
        ) {
            sqlx::query(
                "INSERT INTO v4_registered_contracts \
                   (id, transaction_hash, ledger_sequence, created_at, \
                    channel, contract_name, contract_id, sac, wasm_hash) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(id)
            .bind(format!("tx-{id}"))
            .bind(ledger)
            .bind(ts("2026-04-17 12:00:00"))
            .bind(channel)
            .bind(contract_name)
            .bind(contract_id)
            .bind(sac)
            .bind(wasm_hash)
            .execute(pool)
            .await
            .expect("insert registered");
        }

        async fn insert_deployed(
            pool: &PgPool,
            id: &str,
            ledger: i64,
            channel: &str,
            contract_id: &str,
            deployer: &str,
        ) {
            sqlx::query(
                "INSERT INTO v4_deployed_contracts \
                   (id, transaction_hash, ledger_sequence, created_at, \
                    channel, wasm_name, wasm_version, deployer, contract_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(id)
            .bind(format!("tx-{id}"))
            .bind(ledger)
            .bind(ts("2026-04-17 12:00:00"))
            .bind(channel)
            .bind("ignored")
            .bind("ignored")
            .bind(deployer)
            .bind(contract_id)
            .execute(pool)
            .await
            .expect("insert deployed");
        }

        async fn insert_registry(
            pool: &PgPool,
            contract_id: &str,
            channel_name: &str,
            ledger: i64,
        ) {
            sqlx::query(
                "INSERT INTO v4_registries \
                   (contract_id, channel, id, transaction_hash, ledger_sequence, created_at) \
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(contract_id)
            .bind(channel_name)
            .bind(format!("reg-{contract_id}"))
            .bind(format!("tx-{contract_id}"))
            .bind(ledger)
            .bind(ts("2026-04-17 12:00:00"))
            .execute(pool)
            .await
            .expect("insert registry");
        }

        macro_rules! skip_without_db {
            () => {
                match setup_pool().await {
                    Some(pool) => pool,
                    None => {
                        eprintln!("skipping: TEST_DATABASE_URL not set");
                        return;
                    }
                }
            };
        }

        #[actix_web::test]
        #[serial]
        async fn health_returns_200() {
            let pool = skip_without_db!();
            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/health").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }

        #[actix_web::test]
        #[serial]
        async fn index_lists_v1() {
            let pool = skip_without_db!();
            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            assert_eq!(body["name"], "Registry Indexer API");
            assert_eq!(body["versions"][0]["version"], "v1");
        }

        #[actix_web::test]
        #[serial]
        async fn wasms_list_empty() {
            let pool = skip_without_db!();
            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/wasms").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            assert_eq!(body["result"].as_array().unwrap().len(), 0);
            assert!(body["next"].is_null());
        }

        #[actix_web::test]
        #[serial]
        async fn wasms_list_keeps_latest_per_name() {
            let pool = skip_without_db!();
            // Two versions of "foo", one older "bar". Expect 2 rows, foo@v2 + bar@v1.
            insert_wasm(&pool, "a1", 100, "root", "foo", "1.0.0", "h-foo-1").await;
            insert_wasm(&pool, "a2", 200, "root", "foo", "2.0.0", "h-foo-2").await;
            insert_wasm(&pool, "b1", 150, "root", "bar", "1.0.0", "h-bar-1").await;

            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/wasms").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            let rows = body["result"].as_array().unwrap();
            assert_eq!(rows.len(), 2);
            // Ordered by ledger_sequence ASC in the outer query — bar@150 before foo@200.
            assert_eq!(rows[0]["wasm_name"], "bar");
            assert_eq!(rows[0]["wasm_version"], "1.0.0");
            assert_eq!(rows[1]["wasm_name"], "foo");
            assert_eq!(rows[1]["wasm_version"], "2.0.0");
        }

        #[actix_web::test]
        #[serial]
        async fn wasm_by_name_resolves_friendly_channel() {
            let pool = skip_without_db!();
            // A sub-registry emits from contract_id CSUB…, announced under channel "soroswap".
            insert_registry(&pool, "CSUBREG", "soroswap", 10).await;
            insert_wasm(
                &pool,
                "w1",
                300,
                "CSUBREG",
                "pool",
                "1.0.0",
                "h-pool-1",
            )
            .await;

            let app = make_app!(pool);
            let req = test::TestRequest::get()
                .uri("/v1/wasms/soroswap/pool")
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            assert_eq!(body["wasm_name"], "pool");
            assert_eq!(body["channel"], "soroswap");
            let versions = body["versions"].as_array().unwrap();
            assert_eq!(versions.len(), 1);
        }

        #[actix_web::test]
        #[serial]
        async fn wasm_by_version_exact_match() {
            let pool = skip_without_db!();
            insert_wasm(&pool, "v1", 100, "root", "foo", "1.0.0", "h1").await;
            insert_wasm(&pool, "v2", 200, "root", "foo", "2.0.0", "h2").await;

            let app = make_app!(pool);
            let req = test::TestRequest::get()
                .uri("/v1/wasms/foo/v/1.0.0")
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            assert_eq!(body["wasm_version"], "1.0.0");
            assert_eq!(body["wasm_hash"], "h1");
        }

        #[actix_web::test]
        #[serial]
        async fn wasm_not_found_returns_404() {
            let pool = skip_without_db!();
            let app = make_app!(pool);
            let req = test::TestRequest::get()
                .uri("/v1/wasms/missing")
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }

        #[actix_web::test]
        #[serial]
        async fn wasms_rejects_out_of_range_limit() {
            let pool = skip_without_db!();
            let app = make_app!(pool);

            for q in ["/v1/wasms?limit=0", "/v1/wasms?limit=201"] {
                let req = test::TestRequest::get().uri(q).to_request();
                let resp = test::call_service(&app, req).await;
                assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "query {q}");
            }
        }

        #[actix_web::test]
        #[serial]
        async fn wasms_rejects_malformed_cursor() {
            let pool = skip_without_db!();
            let app = make_app!(pool);
            let req = test::TestRequest::get()
                .uri("/v1/wasms?cursor=not-a-number-hash")
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }

        #[actix_web::test]
        #[serial]
        async fn contracts_list_joins_deployer_and_wasm() {
            let pool = skip_without_db!();
            insert_wasm(&pool, "w1", 50, "root", "token", "1.0.0", "h-token").await;
            insert_registered(
                &pool,
                "r1",
                100,
                "root",
                "usdc",
                "CCONTRACTUSDC",
                "h-token",
                false,
            )
            .await;
            insert_deployed(&pool, "d1", 80, "root", "CCONTRACTUSDC", "GDEPLOYER").await;

            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/contracts").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            let rows = body["result"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["contract_name"], "usdc");
            assert_eq!(rows[0]["contract_id"], "CCONTRACTUSDC");
            assert_eq!(rows[0]["deployer"], "GDEPLOYER");
            assert_eq!(rows[0]["wasm_name"], "token");
            assert_eq!(rows[0]["is_stellar_asset_contract"], false);
        }

        #[actix_web::test]
        #[serial]
        async fn contract_by_name_not_found_returns_404() {
            let pool = skip_without_db!();
            let app = make_app!(pool);
            let req = test::TestRequest::get()
                .uri("/v1/contracts/nope")
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }

        #[actix_web::test]
        #[serial]
        async fn registries_returns_seeded_rows() {
            let pool = skip_without_db!();
            insert_registry(&pool, "CSUB1", "soroswap", 10).await;
            insert_registry(&pool, "CSUB2", "blend", 11).await;

            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/registries").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            let rows = body["result"].as_array().unwrap();
            assert_eq!(rows.len(), 2);
            // Ordered by channel ASC — blend before soroswap.
            assert_eq!(rows[0]["channel"], "blend");
            assert_eq!(rows[1]["channel"], "soroswap");
        }
    }

    // End-to-end: real Soroban testnet events → pipeline transforms → sink
    // tables → fly-app HTTP endpoints. Gated on both TEST_DATABASE_URL and
    // the presence of `test/fixtures/soroban-events-real/` (refresh via
    // `npm run fixtures:refresh` from the repo root). When either is
    // missing, every test in the module returns early with a skip message.
    //
    // Caveat: Goldsky's Turbo runtime is Flink/Calcite, not Postgres. Postgres
    // accepts the same JSON_VALUE / CAST syntax for our current transforms,
    // but dialect drift could pass this test and still break in prod — see
    // test/integration/pipeline-transforms.test.ts for the same caveat.
    mod e2e_real_data {
        use super::{body_json, configure_routes, PgPool};
        use actix_web::{http::StatusCode, test};
        use serde::Deserialize;
        use serial_test::serial;
        use sqlx::postgres::PgPoolOptions;
        use sqlx::Executor;
        use std::collections::HashMap;
        use std::path::PathBuf;

        fn repo_root() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf()
        }

        fn real_fixtures_dir() -> PathBuf {
            repo_root().join("test/fixtures/soroban-events-real")
        }

        fn has_real_fixtures() -> bool {
            let dir = real_fixtures_dir();
            let Ok(entries) = std::fs::read_dir(&dir) else {
                return false;
            };
            entries
                .filter_map(|e| e.ok())
                .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        }

        #[derive(Deserialize)]
        struct Fixture {
            id: String,
            transaction_hash: String,
            ledger_sequence: i64,
            created_at: String,
            command: String,
            channel: String,
            emitter_contract_id: String,
            data: String,
            topics: String,
        }

        #[derive(Deserialize)]
        struct PipelineDoc {
            transforms: HashMap<String, PipelineTransform>,
        }

        #[derive(Deserialize)]
        struct PipelineTransform {
            sql: String,
        }

        fn load_transforms() -> HashMap<String, String> {
            let text = std::fs::read_to_string(repo_root().join("registry-turbo-v4.yaml"))
                .expect("read pipeline yaml");
            let doc: PipelineDoc =
                serde_yaml_ng::from_str(&text).expect("parse pipeline yaml");
            doc.transforms.into_iter().map(|(k, v)| (k, v.sql)).collect()
        }

        fn load_fixtures() -> Vec<Fixture> {
            let mut out = vec![];
            let mut entries: Vec<_> = std::fs::read_dir(real_fixtures_dir())
                .expect("read fixtures dir")
                .filter_map(|e| e.ok())
                .collect();
            entries.sort_by_key(|e| e.path());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let json = std::fs::read_to_string(&path).expect("read fixture");
                let rows: Vec<Fixture> =
                    serde_json::from_str(&json).expect("parse fixture");
                out.extend(rows);
            }
            out
        }

        /// Maps each sink table to (transform_3 name, column list). Must match
        /// the `sinks:` section in registry-turbo-v4.yaml. If a new sink is
        /// added there, extend this list so its rows flow through.
        const SINK_MAPPINGS: &[(&str, &str, &str)] = &[
            (
                "transform_3_deploy_events",
                "v4_deployed_contracts",
                "id, transaction_hash, ledger_sequence, created_at, channel, \
                 wasm_name, wasm_version, deployer, contract_id",
            ),
            (
                "transform_3_publish_events",
                "v4_published_wasms",
                "id, transaction_hash, ledger_sequence, created_at, channel, \
                 author, wasm_version, wasm_hash, wasm_name",
            ),
            (
                "transform_3_register_events",
                "v4_registered_contracts",
                "id, transaction_hash, ledger_sequence, created_at, channel, \
                 contract_name, contract_id, sac, wasm_hash",
            ),
            (
                "transform_3_rename",
                "v4_rename",
                "id, transaction_hash, ledger_sequence, created_at, channel, \
                 old_name, new_name",
            ),
            (
                "transform_3_update_address",
                "v4_update_address",
                "id, transaction_hash, ledger_sequence, created_at, channel, \
                 contract_name, new_address",
            ),
            (
                "transform_3_update_owner",
                "v4_update_owner",
                "id, transaction_hash, ledger_sequence, created_at, channel, \
                 contract_name, new_owner",
            ),
            (
                "transform_3_subregistry_events",
                "v4_registries",
                "id, transaction_hash, ledger_sequence, created_at, \
                 contract_id, channel",
            ),
        ];

        async fn setup_pool_with_real_data() -> Option<PgPool> {
            if !has_real_fixtures() {
                eprintln!("skipping e2e: test/fixtures/soroban-events-real/ empty");
                return None;
            }
            let url = std::env::var("TEST_DATABASE_URL").ok()?;
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .expect("connect to test db");

            let manifest = env!("CARGO_MANIFEST_DIR");
            for rel in [
                "../sql/v4_sink_tables.sql",
                "../sql/v4_registries.sql",
                "../sql/v4_named_views.sql",
            ] {
                let sql = tokio::fs::read_to_string(format!("{manifest}/{rel}"))
                    .await
                    .expect("read sql");
                sqlx::raw_sql(&sql).execute(&pool).await.expect("apply sql");
            }

            pool.execute(
                "TRUNCATE v4_published_wasms, v4_registered_contracts, \
                 v4_deployed_contracts, v4_registries, v4_rename, \
                 v4_update_address, v4_update_owner, v4_raw_events_backup \
                 RESTART IDENTITY",
            )
            .await
            .expect("truncate");

            // Staging table that mirrors the pipeline's transform_2 output.
            // Drop + recreate so reruns against an existing DB start clean.
            pool.execute("DROP TABLE IF EXISTS transform_2_events_with_command_name")
                .await
                .expect("drop staging");
            pool.execute(
                "CREATE TABLE transform_2_events_with_command_name (
                   id TEXT PRIMARY KEY,
                   transaction_hash TEXT,
                   ledger_sequence BIGINT,
                   created_at TIMESTAMP,
                   command TEXT,
                   channel TEXT,
                   emitter_contract_id TEXT,
                   data JSONB,
                   topics JSONB
                 )",
            )
            .await
            .expect("create staging");

            for row in load_fixtures() {
                let ts = chrono::DateTime::parse_from_rfc3339(&row.created_at)
                    .expect("parse created_at")
                    .naive_utc();
                sqlx::query(
                    "INSERT INTO transform_2_events_with_command_name \
                       (id, transaction_hash, ledger_sequence, created_at, command, \
                        channel, emitter_contract_id, data, topics) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8::jsonb,$9::jsonb)",
                )
                .bind(&row.id)
                .bind(&row.transaction_hash)
                .bind(row.ledger_sequence)
                .bind(ts)
                .bind(&row.command)
                .bind(&row.channel)
                .bind(&row.emitter_contract_id)
                .bind(&row.data)
                .bind(&row.topics)
                .execute(&pool)
                .await
                .expect("seed transform_2");
            }

            let transforms = load_transforms();
            for (transform, table, cols) in SINK_MAPPINGS {
                let sql = transforms
                    .get(*transform)
                    .unwrap_or_else(|| panic!("missing transform {transform}"));
                let stmt = format!(
                    "INSERT INTO {table} ({cols}) SELECT {cols} FROM ({sql}) AS src"
                );
                sqlx::raw_sql(&stmt)
                    .execute(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("replay {transform} → {table}: {e}"));
            }

            Some(pool)
        }

        macro_rules! skip_without_e2e {
            () => {
                match setup_pool_with_real_data().await {
                    Some(pool) => pool,
                    None => {
                        eprintln!("skipping e2e: TEST_DATABASE_URL or fixtures missing");
                        return;
                    }
                }
            };
        }

        fn count_command(command: &str) -> usize {
            load_fixtures()
                .iter()
                .filter(|r| r.command == command)
                .count()
        }

        #[actix_web::test]
        #[serial]
        async fn real_sub_regs_reach_registries_endpoint() {
            let pool = skip_without_e2e!();
            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/registries").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            let rows = body["result"].as_array().unwrap();
            // Every fixture sub_reg is emitted by the current root, so every
            // one should pass the pipeline filter. Flipping the root contract
            // id in registry-turbo-v4.yaml would make rows.len() drop to 0.
            let expected = count_command("sub_reg");
            assert!(expected > 0, "no sub_reg events in fixtures");
            assert_eq!(rows.len(), expected, "sub_regs should pass root filter");
            let channels: std::collections::HashSet<_> = rows
                .iter()
                .map(|r| r["channel"].as_str().unwrap().to_string())
                .collect();
            // "root" and "unverified" are the two sub-registry announcements
            // the real root always emits; used here to verify the `name`
            // JSON-path extraction landed the friendly channel string.
            assert!(channels.contains("root"), "channels: {channels:?}");
            assert!(channels.contains("unverified"), "channels: {channels:?}");
        }

        #[actix_web::test]
        #[serial]
        async fn real_registers_reach_contracts_endpoint() {
            let pool = skip_without_e2e!();
            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/contracts").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            let rows = body["result"].as_array().unwrap();
            let expected = count_command("register");
            assert!(expected > 0, "no register events in fixtures");
            assert_eq!(rows.len(), expected);
            for row in rows {
                assert!(
                    !row["contract_name"].as_str().unwrap().is_empty(),
                    "contract_name empty: {row}"
                );
                assert!(
                    row["contract_id"].as_str().unwrap().starts_with('C'),
                    "contract_id malformed: {row}"
                );
                assert!(
                    row["is_stellar_asset_contract"].is_boolean(),
                    "sac missing: {row}"
                );
            }
        }

        #[actix_web::test]
        #[serial]
        async fn real_publish_reaches_wasms_endpoint() {
            let pool = skip_without_e2e!();
            let app = make_app!(pool);
            let req = test::TestRequest::get().uri("/v1/wasms").to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_json(resp).await;
            let rows = body["result"].as_array().unwrap();
            let expected = count_command("publish");
            assert!(expected > 0, "no publish events in fixtures");
            // /v1/wasms groups by wasm_name (latest per name), so a larger
            // publish set may collapse to fewer rows — assert the returned
            // row count is at most the input count but at least 1.
            assert!(!rows.is_empty());
            assert!(rows.len() <= expected);
            for row in rows {
                assert!(row["wasm_hash"].as_str().unwrap().len() >= 32);
                assert!(row["wasm_name"].is_string());
                // The publish events are emitted from the root's contract_id,
                // which v4_published_wasms_named resolves back to "root" via
                // v4_registries. Sanity check the channel lookup wiring.
                assert_eq!(row["channel"], "root");
            }
        }
    }
}
