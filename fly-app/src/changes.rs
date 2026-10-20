use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing_actix_web::RequestId;

use crate::error::{internal_server_error_response, log_db_error, ErrorResponse};

// Ledgers held back from the pipeline's index height, to cover lag between
// the heartbeat sink and the event sinks (~5s per ledger).
const SETTLE_LEDGERS: i64 = 12;
const MAX_LIMIT: i64 = 200;

#[derive(Deserialize)]
pub struct ChangesParams {
    order: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Order {
    Asc,
    Desc,
}

#[derive(sqlx::FromRow, Serialize)]
struct Change {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "ledger")]
    ledger_sequence: i64,
    #[serde(rename = "closed_at")]
    created_at: chrono::NaiveDateTime,
    transaction_hash: String,
    channel: Option<String>,
    event: serde_json::Value,
    wasm: Option<serde_json::Value>,
    contract: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ChangesResponse {
    result: Vec<Change>,
    next: Option<String>,
    current_through: i64,
}

fn bad_request(error: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorResponse {
        error: error.into(),
    })
}

fn parse_order(order: Option<&str>) -> Result<Order, HttpResponse> {
    match order {
        None | Some("asc") => Ok(Order::Asc),
        Some("desc") => Ok(Order::Desc),
        Some(_) => Err(bad_request("order must be asc or desc")),
    }
}

/// Splits an event id (`<ledger>-<tx hash>-op-<n>-event-<n>`) into its
/// ledger and the full id, for a strict (ledger_sequence, id) comparison.
fn parse_event_cursor(cursor: Option<&str>) -> Result<Option<(i64, String)>, HttpResponse> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    match cursor.split_once('-').map(|(l, _)| l.parse::<i64>()) {
        Some(Ok(ledger)) if ledger >= 0 => Ok(Some((ledger, cursor.to_string()))),
        _ => Err(bad_request("Invalid cursor")),
    }
}

/// Ascending pages always return a cursor (the last id seen, or the one
/// passed in) so a client can keep polling for new changes with it.
fn next_cursor(order: Order, rows: &[Change], limit: i64, cursor: Option<&str>) -> Option<String> {
    let last = rows.last().map(|r| r.id.clone());
    match order {
        Order::Asc => last.or_else(|| cursor.map(String::from)),
        Order::Desc if rows.len() as i64 == limit => last,
        Order::Desc => None,
    }
}

async fn current_through(pool: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    let height: Option<i64> =
        sqlx::query_scalar("SELECT ledger_sequence FROM v1.index_height WHERE id = 'v1'")
            .fetch_optional(pool)
            .await?;
    Ok(height.map(|h| (h - SETTLE_LEDGERS).max(0)))
}

#[derive(sqlx::FromRow, Serialize)]
struct IndexHeight {
    #[serde(rename = "ledger")]
    ledger_sequence: i64,
    closed_at: chrono::NaiveDateTime,
}

/// Latest ledger the pipeline has processed, without the feed's settle margin.
pub async fn get_index_height(pool: web::Data<PgPool>, request_id: RequestId) -> HttpResponse {
    let row = sqlx::query_as::<_, IndexHeight>(
        "SELECT ledger_sequence, closed_at FROM v1.index_height WHERE id = 'v1'",
    )
    .fetch_optional(pool.get_ref())
    .await;

    match row {
        Ok(Some(height)) => HttpResponse::Ok().json(height),
        Ok(None) => HttpResponse::ServiceUnavailable().json(ErrorResponse {
            error: "Index height not available yet".into(),
        }),
        Err(e) => {
            log_db_error("get_index_height", &e, pool.get_ref());
            internal_server_error_response(request_id)
        }
    }
}

pub async fn get_changes(
    pool: web::Data<PgPool>,
    query: web::Query<ChangesParams>,
    request_id: RequestId,
) -> HttpResponse {
    let order = match parse_order(query.order.as_deref()) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(MAX_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return bad_request("Limit must be an integer between 1 and 200");
    }
    let cursor = match parse_event_cursor(query.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let current_through = match current_through(pool.get_ref()).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            return HttpResponse::ServiceUnavailable().json(ErrorResponse {
                error: "Index height not available yet".into(),
            })
        }
        Err(e) => {
            log_db_error("get_changes.index_height", &e, pool.get_ref());
            return internal_server_error_response(request_id);
        }
    };

    let (cmp, dir) = match order {
        Order::Asc => (">", "ASC"),
        Order::Desc => ("<", "DESC"),
    };
    let statement = format!(
        "SELECT c.id, c.type AS kind, c.ledger_sequence, c.created_at, c.transaction_hash, \
                reg.registry_channel AS channel, c.event, \
                CASE WHEN c.wasm_hash IS NOT NULL THEN jsonb_build_object( \
                    'wasm_hash', c.wasm_hash, 'wasm_name', c.event->>'wasm_name', \
                    'wasm_version', c.event->>'wasm_version', 'author', c.event->>'author', \
                    'channel', reg.registry_channel) END AS wasm, \
                CASE WHEN cr.registration_id IS NOT NULL THEN to_jsonb(cr) END AS contract \
         FROM v1.changes c \
         LEFT JOIN v1.registries reg ON reg.contract_id = c.emitter_contract_id \
         LEFT JOIN v1.contract_registrations cr ON cr.registration_id = c.registration_id \
         WHERE c.ledger_sequence <= $1 \
           AND ($2::bigint IS NULL OR (c.ledger_sequence, c.id) {cmp} ($2, $3)) \
         ORDER BY c.ledger_sequence {dir}, c.id {dir} \
         LIMIT $4"
    );

    let (cursor_ledger, cursor_id) = cursor.unzip();
    let rows = sqlx::query_as::<_, Change>(&statement)
        .bind(current_through)
        .bind(cursor_ledger)
        .bind(cursor_id)
        .bind(limit)
        .fetch_all(pool.get_ref())
        .await;

    match rows {
        Ok(rows) => {
            let next = next_cursor(order, &rows, limit, query.cursor.as_deref());
            HttpResponse::Ok().json(ChangesResponse {
                result: rows,
                next,
                current_through,
            })
        }
        Err(e) => {
            log_db_error("get_changes.fetch_changes", &e, pool.get_ref());
            internal_server_error_response(request_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_order() {
        assert_eq!(parse_order(None).ok(), Some(Order::Asc));
        assert_eq!(parse_order(Some("asc")).ok(), Some(Order::Asc));
        assert_eq!(parse_order(Some("desc")).ok(), Some(Order::Desc));
        assert!(parse_order(Some("DESC")).is_err());
    }

    #[test]
    fn parses_event_cursor() {
        let id = "2812868-51b6-op-0-event-0";
        assert_eq!(
            parse_event_cursor(Some(id)).ok().flatten(),
            Some((2812868, id.to_string()))
        );
        assert!(parse_event_cursor(None).ok().flatten().is_none());
        for bad in ["", "abc-def", "-1-x", "123"] {
            assert!(parse_event_cursor(Some(bad)).is_err(), "{bad}");
        }
    }

    // Runs the real post_init.sql views against a scratch database. Skipped
    // unless TEST_DATABASE_URL is set; it drops and recreates the v1 and
    // archive schemas there.
    mod db {
        use super::super::*;
        use actix_web::{http::StatusCode, test as actix_test, App};
        use serde_json::Value;
        use sqlx::postgres::PgPoolOptions;
        use tracing_actix_web::TracingLogger;

        const SEED: &str = "
            INSERT INTO v1.registries VALUES ('0-r', 'tx0', 1, now(), 'CMAIN', 'main');
            INSERT INTO v1.published_wasms VALUES
              ('100-tx100-op-0-event-0', 'tx100', 100, now(), 'CMAIN', 'GAUTHOR', '1.0.0', 'hash1', 'token');
            INSERT INTO v1.deployed_contracts VALUES
              ('110-tx110-op-0-event-0', 'tx110', 110, now(), 'CMAIN', 'token', '1.0.0', 'GDEPLOYER', 'CA', 'CMAIN');
            INSERT INTO v1.registered_contracts VALUES
              ('110-tx110-op-0-event-1', 'tx110', 110, now(), 'CMAIN', 'alpha', 'CA', false, 'hash1'),
              ('130-tx130-op-0-event-0', 'tx130', 130, now(), 'CMAIN', 'alpha', 'CB', false, NULL);
            INSERT INTO v1.rename VALUES
              ('120-tx120-op-0-event-0', 'tx120', 120, now(), 'CMAIN', 'alpha', 'beta'),
              ('160-tx160-op-0-event-0', 'tx160', 160, now(), 'CMAIN', 'beta', 'gamma');
            INSERT INTO v1.update_address VALUES
              ('140-tx140-op-0-event-0', 'tx140', 140, now(), 'CMAIN', 'beta', 'CC');
            INSERT INTO v1.update_owner VALUES
              ('150-tx150-op-0-event-0', 'tx150', 150, now(), 'CMAIN', 'alpha', 'GOWNER'),
              ('165-tx165-op-0-event-0', 'tx165', 165, now(), 'CMAIN', 'gamma', 'GLATE');
            INSERT INTO archive.deploys VALUES
              ('110-d', 110, now(), 'tx110', 'CA', 'hash1'),
              ('140-d', 139, now(), 'tx139', 'CC', 'hash1');
            INSERT INTO v1.index_height VALUES ('v1', 172, now());
        ";

        async fn pool() -> Option<PgPool> {
            let url = std::env::var("TEST_DATABASE_URL").ok()?;
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(&url)
                .await
                .unwrap();
            sqlx::raw_sql(
                "DROP SCHEMA IF EXISTS v1 CASCADE; DROP SCHEMA IF EXISTS archive CASCADE;",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::raw_sql(include_str!("../tests/fixtures/goldsky_tables.sql"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::raw_sql(include_str!("../../goldsky/v1/post_init.sql"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::raw_sql(SEED).execute(&pool).await.unwrap();
            Some(pool)
        }

        async fn get(pool: &PgPool, uri: &str) -> (StatusCode, Value) {
            let app = actix_test::init_service(
                App::new()
                    .wrap(TracingLogger::default())
                    .app_data(web::Data::new(pool.clone()))
                    .route("/v1/changes", web::get().to(get_changes))
                    .route("/v1/index-height", web::get().to(get_index_height)),
            )
            .await;
            let req = actix_test::TestRequest::get().uri(uri).to_request();
            let resp = actix_test::call_service(&app, req).await;
            let status = resp.status();
            let body = actix_test::read_body(resp).await;
            (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
        }

        fn types(body: &Value) -> Vec<&str> {
            body["result"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["type"].as_str().unwrap())
                .collect()
        }

        // One test so the shared schema isn't rebuilt under a concurrent test.
        #[actix_web::test]
        async fn change_feed() {
            let Some(pool) = pool().await else {
                eprintln!("TEST_DATABASE_URL not set; skipping");
                return;
            };

            let (status, all) = get(&pool, "/v1/changes").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(all["current_through"], 160);
            let (status, height) = get(&pool, "/v1/index-height").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(height["ledger"], 172);
            // The ledger-165 update_owner is past current_through.
            assert_eq!(
                types(&all),
                [
                    "publish",
                    "deploy",
                    "register",
                    "rename",
                    "register",
                    "update_address",
                    "update_owner",
                    "rename"
                ]
            );
            let r = &all["result"];
            assert_eq!(all["next"], r[7]["id"]);

            assert_eq!(r[0]["wasm"]["wasm_name"], "token");
            assert_eq!(r[0]["wasm"]["channel"], "main");
            assert!(r[0]["contract"].is_null());

            // The first registration, followed through two renames and an
            // address change; every event about it carries its current state.
            for i in [1, 2, 3, 5, 7] {
                let c = &r[i]["contract"];
                assert_eq!(c["registration_id"], "110-tx110-op-0-event-1", "entry {i}");
                assert_eq!(c["contract_name"], "gamma");
                assert_eq!(c["contract_id"], "CC");
                assert_eq!(c["wasm_name"], "token");
                assert_eq!(c["deployer"], "GDEPLOYER");
            }
            assert_eq!(r[3]["event"]["new_name"], "beta");

            // "alpha" was reused by a second registration after the rename;
            // the owner change on it belongs to that one.
            for i in [4, 6] {
                let c = &r[i]["contract"];
                assert_eq!(c["registration_id"], "130-tx130-op-0-event-0", "entry {i}");
                assert_eq!(c["contract_name"], "alpha");
                assert_eq!(c["contract_id"], "CB");
            }
            assert_eq!(r[6]["contract"]["owner"], "GOWNER");

            // Paging forward reproduces the full list.
            let (_, page1) = get(&pool, "/v1/changes?limit=3").await;
            let cursor = page1["next"].as_str().unwrap().to_string();
            let (_, page2) = get(&pool, &format!("/v1/changes?cursor={cursor}")).await;
            assert_eq!([types(&page1), types(&page2)].concat(), types(&all));
            // An empty poll hands the cursor back.
            let last = all["next"].as_str().unwrap();
            let (_, empty) = get(&pool, &format!("/v1/changes?cursor={last}")).await;
            assert_eq!(types(&empty).len(), 0);
            assert_eq!(empty["next"], last);

            let (_, desc) = get(&pool, "/v1/changes?order=desc&limit=2").await;
            assert_eq!(types(&desc), ["rename", "update_owner"]);
            let cursor = desc["next"].as_str().unwrap().to_string();
            let (_, desc2) = get(
                &pool,
                &format!("/v1/changes?order=desc&limit=2&cursor={cursor}"),
            )
            .await;
            assert_eq!(types(&desc2), ["update_address", "register"]);

            let (status, _) = get(&pool, "/v1/changes?limit=201").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let (status, _) = get(&pool, "/v1/changes?cursor=nope").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);

            sqlx::query("TRUNCATE v1.index_height")
                .execute(&pool)
                .await
                .unwrap();
            let (status, _) = get(&pool, "/v1/changes").await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            let (status, _) = get(&pool, "/v1/index-height").await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        }
    }
}
