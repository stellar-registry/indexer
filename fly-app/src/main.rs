use actix_web::{web, App, HttpResponse, HttpServer};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

#[derive(Deserialize)]
struct QueryParams {
    limit: Option<i64>,
    cursor: Option<String>,
}

/// Table "publishes_5"
///       Column      |            Type             | Collation | Nullable | Default
/// ------------------+-----------------------------+-----------+----------+---------
///  id               | text                        |           | not null |
///  transaction_hash | text                        |           | not null |
///  ledger_sequence  | bigint                      |           | not null |
///  created_at       | timestamp without time zone |           | not null |
///  author           | text                        |           |          |
///  version          | text                        |           |          |
///  wasm_name        | text                        |           |          |
///  wasm_hash        | text                        |           |          |
///
#[derive(sqlx::FromRow)]
struct WasmRow {
    id: String,
    transaction_hash: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
    author: Option<String>,
    version: Option<String>,
    wasm_name: Option<String>,
    wasm_hash: Option<String>,
}

/// Table "deploys_5"
///       Column      |            Type             | Collation | Nullable | Default
/// ------------------+-----------------------------+-----------+----------+---------
///  id               | text                        |           | not null |
///  transaction_hash | text                        |           | not null |
///  ledger_sequence  | bigint                      |           | not null |
///  created_at       | timestamp without time zone |           | not null |
///  contract_id      | text                        |           |          |
///  contract_name    | text                        |           |          |
///  deployer         | text                        |           |          |
///  version          | text                        |           |          |
///  wasm_name        | text                        |           |          |
///
#[derive(sqlx::FromRow)]
struct ContractRow {
    id: String,
    transaction_hash: String,
    ledger_sequence: i64,
    created_at: chrono::NaiveDateTime,
    contract_id: Option<String>,
    contract_name: Option<String>,
    deployer: Option<String>,
    version: Option<String>,
    wasm_name: Option<String>,
}

#[derive(Serialize)]
struct WasmResult {
    author: Option<String>,
    version: Option<String>,
    wasm_name: Option<String>,
    wasm_hash: Option<String>,
}

#[derive(Serialize)]
struct Response {
    result: Vec<WasmResult>,
    next: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn get_wasms(pool: web::Data<PgPool>, query: web::Query<QueryParams>) -> HttpResponse {
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

    let rows = sqlx::query_as::<_, WasmRow>(
        "SELECT id, author, version, wasm_name, wasm_hash FROM \
           (SELECT *, ROW_NUMBER() OVER \
             (PARTITION BY wasm_name ORDER BY ledger_sequence, version DESC) AS rn \
             FROM public.publishes_5 \
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

            let result: Vec<WasmResult> = rows
                .into_iter()
                .map(|r| WasmResult {
                    author: r.author,
                    version: r.version,
                    wasm_name: r.wasm_name,
                    wasm_hash: r.wasm_hash,
                })
                .collect();

            HttpResponse::Ok().json(Response { result, next })
        }
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

    // Append 'z' to make the cursor lexicographically greater, advancing past
    // the current transaction within the same ledger.
    let cursor = format!("{}-{}-z", parts[0], parts[1]);
    Ok((ledger, cursor))
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
            .route("/", web::get().to(get_wasms))
            .route("/health", web::get().to(health))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
