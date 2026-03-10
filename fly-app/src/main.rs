use actix_web::{web, App, HttpResponse, HttpServer};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

#[derive(Deserialize)]
struct QueryParams {
    limit: Option<i64>,
    cursor: Option<String>,
}

/// Slim result for /wasms list endpoint and versions array
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

/// DB row mapping for v1_published_wasms
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
    versions: Vec<WasmResult>,
}

/// Slim result for /contracts list endpoint
#[derive(sqlx::FromRow, Serialize)]
struct ContractResult {
    #[serde(skip)]
    id: String,
    contract_id: Option<String>,
    contract_name: Option<String>,
    deployer: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
}

/// Full detail for /contracts/{contract_name} endpoint
///
/// From table "v1_deployed_contracts":
///
/// ```
/// Column      |            Type             | Collation | Nullable | Default
/// ------------------+-----------------------------+-----------+----------+---------
/// id               | text                        |           | not null |
/// transaction_hash | text                        |           | not null |
/// ledger_sequence  | bigint                      |           | not null |
/// created_at       | timestamp without time zone |           | not null |
/// channel    | text                        |           |          |
/// wasm_name        | text                        |           |          |
/// wasm_version     | text                        |           |          |
/// deployer         | text                        |           |          |
/// contract_id      | text                        |           |          |
/// ```
/// And table `v1_registered_wasms`
///       Column      |            Type             | Collation | Nullable | Default
/// ------------------+-----------------------------+-----------+----------+---------
///  id               | text                        |           | not null |
///  transaction_hash | text                        |           | not null |
///  ledger_sequence  | bigint                      |           | not null |
///  created_at       | timestamp without time zone |           | not null |
///  channel    | text                        |           |          |
///  contract_name    | text                        |           |          |
///  contract_id      | text                        |           |          |
#[derive(sqlx::FromRow, Serialize)]
struct ContractDetail {
    id: String,
    transaction_hash: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
    contract_id: Option<String>,
    contract_name: Option<String>,
    deployer: Option<String>,
    wasm_version: Option<String>,
    wasm_name: Option<String>,
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

#[deprecated = "to be removed get_wasms_by_type"]
async fn get_wasms_legacy(pool: web::Data<PgPool>, query: web::Query<QueryParams>) -> HttpResponse {
    get_wasms("unverified", pool, query).await
}

async fn get_wasms_by_type(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    query: web::Query<QueryParams>,
) -> HttpResponse {
    let channel = path.into_inner();
    if channel != "main" && channel != "unverified" {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
        });
    }
    get_wasms(&channel, pool, query).await
}

async fn get_wasms(
    channel: &str,
    pool: web::Data<PgPool>,
    query: web::Query<QueryParams>,
) -> HttpResponse {
    let limit = query.limit.unwrap_or(200);
    if limit < 2 || limit > 200 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
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
        "SELECT id, author, wasm_version, wasm_name, wasm_hash, channel FROM \
           (SELECT *, ROW_NUMBER() OVER \
             (PARTITION BY wasm_name ORDER BY ledger_sequence DESC, wasm_version DESC) AS rn \
             FROM public.v1_published_wasms \
           ) AS sub \
         WHERE rn = 1 AND (ledger_sequence, id) >= ($1, $2) \
         AND channel = $3 \
         ORDER BY ledger_sequence, id ASC \
         LIMIT $4",
    )
    .bind(ledger)
    .bind(&cursor)
    .bind(channel)
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
             FROM public.v1_published_wasms \
             WHERE wasm_name = $1 AND wasm_version = $2 AND channel = $3",
        )
        .bind(wasm_name)
        .bind(ver)
        .bind(channel)
        .fetch_optional(pool)
        .await
    } else {
        sqlx::query_as::<_, WasmDetailRow>(
            "SELECT id, transaction_hash, ledger_sequence, created_at, \
                    author, wasm_version, wasm_name, wasm_hash FROM \
               (SELECT *, ROW_NUMBER() OVER \
                 (PARTITION BY wasm_name ORDER BY ledger_sequence DESC, wasm_version DESC) AS rn \
                 FROM public.v1_published_wasms \
               ) AS sub \
             WHERE rn = 1 AND wasm_name = $1 AND channel = $2",
        )
        .bind(wasm_name)
        .bind(channel)
        .fetch_optional(pool)
        .await
    };

    match row {
        // TODO: can do only one select and filter the results
        Ok(Some(detail_row)) => {
            let versions = sqlx::query_as::<_, WasmResult>(
                "SELECT id, author, wasm_version, wasm_name, wasm_hash \
                 FROM public.v1_published_wasms \
                 WHERE wasm_name = $1 AND channel = $2 \
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

#[deprecated = "to be removed get_wasm_latest"]
async fn get_wasm_latest_legacy(pool: web::Data<PgPool>, path: web::Path<String>) -> HttpResponse {
    let wasm_name = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), "unverified", &wasm_name, None).await
}

async fn get_wasm_latest(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (channel, wasm_name) = path.into_inner();
    if channel != "main" && channel != "unverified" {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
        });
    }
    fetch_wasm_detail(pool.get_ref(), &channel, &wasm_name, None).await
}

#[deprecated = "to be removed get_wasm_version"]
async fn get_wasm_version_legacy(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (wasm_name, version) = path.into_inner();
    fetch_wasm_detail(pool.get_ref(), "unverified", &wasm_name, Some(&version)).await
}

async fn get_wasm_version(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String, String)>,
) -> HttpResponse {
    let (channel, wasm_name, version) = path.into_inner();
    if channel != "main" && channel != "unverified" {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
        });
    }
    fetch_wasm_detail(pool.get_ref(), &channel, &wasm_name, Some(&version)).await
}

#[deprecated = "to be removed get_contracts"]
async fn get_contracts_legacy(
    pool: web::Data<PgPool>,
    query: web::Query<QueryParams>,
) -> HttpResponse {
    fetch_contracts("unverified", pool, query).await
}

async fn get_contracts(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    query: web::Query<QueryParams>,
) -> HttpResponse {
    let channel = path.into_inner();
    if channel != "main" && channel != "unverified" {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
        });
    }
    fetch_contracts(&channel, pool, query).await
}

async fn fetch_contracts(
    channel: &str,
    pool: web::Data<PgPool>,
    query: web::Query<QueryParams>,
) -> HttpResponse {
    let limit = query.limit.unwrap_or(200);
    if limit < 2 || limit > 200 {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
        });
    }

    let (ledger, cursor) = match parse_cursor(&query.cursor) {
        Ok(val) => val,
        Err(resp) => return resp,
    };

    let rows = sqlx::query_as::<_, ContractResult>(
        "SELECT
                dc.id,
                dc.contract_id,
                rw.contract_name,
                dc.deployer,
                dc.wasm_version,
                dc.wasm_name
            FROM public.v1_deployed_contracts dc
            LEFT JOIN public.v1_registered_wasms rw
              ON dc.contract_id = rw.contract_id
            WHERE (dc.ledger_sequence, dc.id) >= ($1, $2) \
            AND dc.channel = $3 \
            ORDER BY dc.ledger_sequence, dc.id ASC \
            LIMIT $4",
    )
    .bind(ledger)
    .bind(&cursor)
    .bind(&channel)
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

#[deprecated = "to be removed get_contracts"]
async fn get_single_contract_legacy(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    let contract_name = path.into_inner();

    fetch_single_contract("unverified", &contract_name, pool).await
}

async fn get_single_contract(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (channel, contract_name) = path.into_inner();
    if channel != "main" && channel != "unverified" {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 2 and 200".into(),
        });
    }
    fetch_single_contract(&channel, &contract_name, pool).await
}

async fn fetch_single_contract(
    channel: &str,
    contract_name: &str,
    pool: web::Data<PgPool>,
) -> HttpResponse {
    let row = sqlx::query_as::<_, ContractDetail>(
        "SELECT
                dc.id,
                dc.transaction_hash,
                dc.ledger_sequence,
                dc.created_at,
                dc.contract_id,
                rw.contract_name,
                dc.deployer,
                dc.wasm_version,
                dc.wasm_name
            FROM public.v1_deployed_contracts dc
            LEFT JOIN public.v1_registered_wasms rw
              ON dc.contract_id = rw.contract_id
            WHERE contract_name = $1 AND dc.channel = $2",
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
    let cursor = format!("{}-{}-z", parts[0], parts[1]);
    Ok((ledger, cursor))
}

async fn index() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "name": "Registry Indexer API",
        "endpoints": [
            { "method": "GET", "path": "/v1/{channel}/wasms", "description": "List published wasms, where channel is either 'main' or 'unverified'" },
            { "method": "GET", "path": "/v1/{channel}/wasms/{wasm_name}", "description": "Get details for the latest version of a specific wasm" },
            { "method": "GET", "path": "/v1/{channel}/wasms/{wasm_name}/v/{version}", "description": "Get details for a specific version of a wasm" },
            { "method": "GET", "path": "/v1/{channel}/contracts", "description": "List deployed contracts" },
            { "method": "GET", "path": "/v1/{channel}/contracts/{contract_name}", "description": "Get details for a specific contract" },
            { "method": "GET", "path": "/health", "description": "Health check" }
        ]
    }))
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
            .route("/wasms", web::get().to(get_wasms_legacy))
            .route("/v1/{channel}/wasms", web::get().to(get_wasms_by_type))
            .route("/wasms/{wasm_name}", web::get().to(get_wasm_latest_legacy))
            .route(
                "/v1/{channel}/wasms/{wasm_name}",
                web::get().to(get_wasm_latest),
            )
            .route(
                "/wasms/{wasm_name}/v/{version}",
                web::get().to(get_wasm_version_legacy),
            )
            .route(
                "/v1/{channel}/wasms/{wasm_name}/v/{version}",
                web::get().to(get_wasm_version),
            )
            .route("/contracts", web::get().to(get_contracts_legacy))
            .route("/v1/{channel}/contracts", web::get().to(get_contracts))
            .route(
                "/contracts/{contract_name}",
                web::get().to(get_single_contract_legacy),
            )
            .route(
                "/v1/{channel}/contracts/{contract_name}",
                web::get().to(get_single_contract),
            )
            .route("/health", web::get().to(health))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
