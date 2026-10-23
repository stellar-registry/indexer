use std::collections::BTreeMap;

use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use serde_qs::actix::QsQuery;
use sqlx::PgPool;
use tracing_actix_web::RequestId;

use crate::error::{internal_server_error_response, log_db_error, ErrorResponse};
use crate::{parse_cursor, util, ListResponse, QueryParams};

/// Most addresses one batch lookup accepts.
const MAX_BATCH_ADDRESSES: usize = 100;

const ACCOUNT_COLUMNS: &str = "id, registration_id, channel, account_name, address, owner, \
                               flagged, created_at, updated_at";

/// Current state of a named account, from the v1.accounts view.
/// registration_id (the register_account event id) stays the same through
/// renames and address changes.
#[derive(sqlx::FromRow, Serialize)]
pub struct AccountResult {
    #[serde(skip)]
    id: String,
    registration_id: String,
    channel: Option<String>,
    account_name: Option<String>,
    address: Option<String>,
    owner: Option<String>,
    flagged: bool,
    created_at: chrono::NaiveDateTime,
    updated_at: Option<chrono::NaiveDateTime>,
}

pub async fn get_accounts_root(
    pool: web::Data<PgPool>,
    query: QsQuery<QueryParams>,
    request_id: RequestId,
) -> HttpResponse {
    let sort_by: Vec<String> = query.sort_by.clone().unwrap_or_default();
    let descending: Vec<bool> = query.descending.clone().unwrap_or_default();
    let limit = query.limit.unwrap_or(200);

    if !(1..=200).contains(&limit) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Limit must be an integer between 1 and 200".into(),
        });
    }

    let (ledger, cursor) = match parse_cursor(&query.cursor) {
        Ok(val) => val,
        Err(resp) => return resp,
    };

    let mut statement = format!(
        "SELECT {ACCOUNT_COLUMNS}, \
                CASE \
                    WHEN $4::text IS NULL THEN 0 \
                    ELSE GREATEST( \
                        v1.similarity(account_name, $4), \
                        v1.similarity(channel, $4), \
                        CASE WHEN address = $4 OR owner = $4 THEN 1.0 ELSE 0.0 END \
                    ) \
                END AS rank \
         FROM v1.accounts \
         WHERE (ledger_sequence, id) >= ($1, $2) \
            AND ( \
                $4::text IS NULL \
                OR v1.similarity(account_name, $4) > 0.2 \
                OR v1.similarity(channel, $4) > 0.2 \
                OR address = $4 \
                OR owner = $4 \
            ) \
         ORDER BY "
    );

    let sort_stmt = match util::build_sort_spec(
        sort_by,
        descending,
        &["account_name", "channel", "address", "owner", "flagged"],
    ) {
        Ok(s) => s,
        Err(e) => return e,
    };

    if !sort_stmt.is_empty() {
        statement.push_str(&sort_stmt);
        statement.push_str(", ");
    }
    statement.push_str("rank DESC, ledger_sequence, id ASC\nLIMIT $3");

    let rows = sqlx::query_as::<_, AccountResult>(&statement)
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
            log_db_error("get_accounts_root.fetch_accounts", &e, pool.get_ref());
            internal_server_error_response(request_id)
        }
    }
}

pub async fn get_single_account_root(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    request_id: RequestId,
) -> HttpResponse {
    fetch_single_account("root", &path.into_inner(), pool.get_ref(), request_id).await
}

pub async fn get_single_account(
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
    request_id: RequestId,
) -> HttpResponse {
    let (channel, account_name) = path.into_inner();
    fetch_single_account(&channel, &account_name, pool.get_ref(), request_id).await
}

async fn fetch_single_account(
    channel: &str,
    account_name: &str,
    pool: &PgPool,
    request_id: RequestId,
) -> HttpResponse {
    let row = sqlx::query_as::<_, AccountResult>(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM v1.accounts \
         WHERE account_name = $1 AND channel = $2 \
         ORDER BY ledger_sequence DESC, id DESC \
         LIMIT 1"
    ))
    .bind(account_name)
    .bind(channel)
    .fetch_optional(pool)
    .await;

    match row {
        Ok(Some(r)) => HttpResponse::Ok().json(r),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Account '{account_name}' not found"),
        }),
        Err(e) => {
            log_db_error("fetch_single_account.select_account", &e, pool);
            internal_server_error_response(request_id)
        }
    }
}

async fn accounts_for_addresses(
    pool: &PgPool,
    addresses: &[String],
) -> Result<Vec<AccountResult>, sqlx::Error> {
    sqlx::query_as::<_, AccountResult>(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM v1.accounts \
         WHERE address = ANY($1) \
         ORDER BY address, channel, account_name"
    ))
    .bind(addresses)
    .fetch_all(pool)
    .await
}

/// Every name currently pointing at one address, across registries.
pub async fn get_accounts_by_address(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    request_id: RequestId,
) -> HttpResponse {
    let address = path.into_inner();
    match accounts_for_addresses(pool.get_ref(), &[address]).await {
        Ok(rows) => HttpResponse::Ok().json(ListResponse {
            result: rows,
            next: None,
        }),
        Err(e) => {
            log_db_error(
                "get_accounts_by_address.select_accounts",
                &e,
                pool.get_ref(),
            );
            internal_server_error_response(request_id)
        }
    }
}

#[derive(Deserialize)]
pub struct BatchQuery {
    addresses: Option<String>,
}

#[derive(Serialize)]
struct BatchResponse {
    /// Every requested address is a key; unnamed ones map to [].
    result: BTreeMap<String, Vec<AccountResult>>,
}

/// Names for many addresses at once: `?addresses=G...,C...` (comma-separated).
pub async fn get_accounts_by_addresses(
    pool: web::Data<PgPool>,
    query: web::Query<BatchQuery>,
    request_id: RequestId,
) -> HttpResponse {
    let addresses = match parse_addresses(query.addresses.as_deref()) {
        Ok(a) => a,
        Err(error) => return HttpResponse::BadRequest().json(ErrorResponse { error }),
    };

    let rows = match accounts_for_addresses(pool.get_ref(), &addresses).await {
        Ok(rows) => rows,
        Err(e) => {
            log_db_error(
                "get_accounts_by_addresses.select_accounts",
                &e,
                pool.get_ref(),
            );
            return internal_server_error_response(request_id);
        }
    };

    let mut result: BTreeMap<String, Vec<AccountResult>> =
        addresses.into_iter().map(|a| (a, Vec::new())).collect();
    for row in rows {
        if let Some(names) = row.address.clone().and_then(|a| result.get_mut(&a)) {
            names.push(row);
        }
    }
    HttpResponse::Ok().json(BatchResponse { result })
}

fn parse_addresses(raw: Option<&str>) -> Result<Vec<String>, String> {
    let mut addresses: Vec<String> = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(String::from)
        .collect();
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return Err("addresses must list at least one address".into());
    }
    if addresses.len() > MAX_BATCH_ADDRESSES {
        return Err(format!(
            "addresses accepts at most {MAX_BATCH_ADDRESSES} addresses"
        ));
    }
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addresses() {
        assert_eq!(
            parse_addresses(Some("GB, GA,,GB")).unwrap(),
            ["GA".to_string(), "GB".to_string()]
        );
        assert!(parse_addresses(None).is_err());
        assert!(parse_addresses(Some(" , ")).is_err());
        let many = vec!["G"; MAX_BATCH_ADDRESSES + 1]
            .iter()
            .enumerate()
            .map(|(i, g)| format!("{g}{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_addresses(Some(&many)).is_err());
    }

    // Runs the real post_init.sql views against a scratch database; see
    // test_db. Skipped unless TEST_DATABASE_URL is set.
    mod db {
        use super::super::*;
        use actix_web::{http::StatusCode, test as actix_test, App};
        use serde_json::Value;
        use tracing_actix_web::TracingLogger;

        // GALICE registers "alice" (owner GALICE), renames it to "alicia",
        // flags it, then points it at GALICE2. "alice" is then taken again by
        // GBOB in the unverified registry. GCAROL has a pending preauth;
        // GDAVE's was revoked; GERIN's was consumed by a registration; GFAY's
        // names another owner than the registration used, so it's still
        // pending.
        const SEED: &str = "
            INSERT INTO v1.registries VALUES
              ('0-r', 'tx0', 1, now(), 'CROOT', 'root'),
              ('1-r', 'tx1', 1, now(), 'CUNV', 'unverified');
            INSERT INTO v1.registered_accounts VALUES
              ('100-tx100-op-0-event-0', 'tx100', 100, '2026-01-01', 'CROOT', 'alice', 'GALICE', 'GALICE'),
              ('110-tx110-op-0-event-0', 'tx110', 110, '2026-01-02', 'CUNV', 'alice', 'GBOB', 'GBOB'),
              ('115-tx115-op-0-event-0', 'tx115', 115, '2026-01-03', 'CROOT', 'erin', 'GERIN', 'GOWNER'),
              ('118-tx118-op-0-event-0', 'tx118', 118, '2026-01-03', 'CROOT', 'fay', 'GFAY', 'GOWNER'),
              ('119-tx119-op-0-event-0', 'tx119', 119, '2026-01-03', 'CROOT', 'old', 'GOLD', NULL);
            INSERT INTO v1.rename_account VALUES
              ('120-tx120-op-0-event-0', 'tx120', 120, '2026-01-04', 'CROOT', 'alice', 'alicia', 'GALICE');
            INSERT INTO v1.sec_flag_acct VALUES
              ('130-tx130-op-0-event-0', 'tx130', 130, '2026-01-05', 'CROOT', 'alicia', true, 'GALICE');
            INSERT INTO v1.update_acct_addr VALUES
              ('140-tx140-op-0-event-0', 'tx140', 140, '2026-01-06', 'CROOT', 'alicia', 'GALICE2', 'GALICE');
            INSERT INTO v1.update_acct_owner VALUES
              ('150-tx150-op-0-event-0', 'tx150', 150, '2026-01-07', 'CROOT', 'old', 'GNEWOWNER', 'GOLD');
            INSERT INTO v1.preauth_acct VALUES
              ('105-tx105-op-0-event-0', 'tx105', 105, now(), 'CROOT', 'carol', 'GCAROL', 'GOWNER'),
              ('106-tx106-op-0-event-0', 'tx106', 106, now(), 'CROOT', 'dave', 'GDAVE', 'GOWNER'),
              ('107-tx107-op-0-event-0', 'tx107', 107, now(), 'CROOT', 'erin', 'GERIN', 'GOWNER'),
              ('108-tx108-op-0-event-0', 'tx108', 108, now(), 'CROOT', 'fay', 'GFAY', 'GSOMEONE');
            INSERT INTO v1.revoke_preauth_acct VALUES
              ('109-tx109-op-0-event-0', 'tx109', 109, now(), 'CROOT', 'dave', 'GDAVE');
        ";

        async fn get(pool: &PgPool, uri: &str) -> (StatusCode, Value) {
            let app = actix_test::init_service(
                App::new()
                    .wrap(TracingLogger::default())
                    .app_data(web::Data::new(pool.clone()))
                    .route("/v1/accounts", web::get().to(get_accounts_root))
                    .route(
                        "/v1/accounts/by-address",
                        web::get().to(get_accounts_by_addresses),
                    )
                    .route(
                        "/v1/accounts/by-address/{address}",
                        web::get().to(get_accounts_by_address),
                    )
                    .route(
                        "/v1/accounts/{account_name}",
                        web::get().to(get_single_account_root),
                    )
                    .route(
                        "/v1/accounts/{channel}/{account_name}",
                        web::get().to(get_single_account),
                    ),
            )
            .await;
            let req = actix_test::TestRequest::get().uri(uri).to_request();
            let resp = actix_test::call_service(&app, req).await;
            let status = resp.status();
            let body = actix_test::read_body(resp).await;
            (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
        }

        fn names(body: &Value) -> Vec<&str> {
            body["result"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["account_name"].as_str().unwrap())
                .collect()
        }

        #[actix_web::test]
        async fn accounts_api_and_views() {
            let Some((pool, _guard)) = crate::test_db::pool(SEED).await else {
                eprintln!("TEST_DATABASE_URL not set; skipping");
                return;
            };

            let (status, alicia) = get(&pool, "/v1/accounts/alicia").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(alicia["registration_id"], "100-tx100-op-0-event-0");
            assert_eq!(alicia["channel"], "root");
            assert_eq!(alicia["address"], "GALICE2");
            assert_eq!(alicia["owner"], "GALICE");
            assert_eq!(alicia["flagged"], true);
            assert_eq!(alicia["updated_at"], "2026-01-06T00:00:00");

            // The old name now resolves only in the registry that reused it.
            let (status, _) = get(&pool, "/v1/accounts/alice").await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            let (_, bob) = get(&pool, "/v1/accounts/unverified/alice").await;
            assert_eq!(bob["address"], "GBOB");
            assert_eq!(bob["flagged"], false);

            // Pre-0.8.0 registrations have no owner until it's updated.
            let (_, old) = get(&pool, "/v1/accounts/old").await;
            assert_eq!(old["owner"], "GNEWOWNER");

            let (status, all) = get(&pool, "/v1/accounts").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(names(&all), ["alicia", "alice", "erin", "fay", "old"]);
            let (_, page1) = get(&pool, "/v1/accounts?limit=2").await;
            let cursor = page1["next"].as_str().unwrap().to_string();
            let (_, page2) = get(&pool, &format!("/v1/accounts?cursor={cursor}")).await;
            assert_eq!([names(&page1), names(&page2)].concat(), names(&all));
            let (_, by_owner) = get(&pool, "/v1/accounts?query=GOWNER").await;
            assert_eq!(names(&by_owner), ["erin", "fay"]);
            let (status, _) = get(&pool, "/v1/accounts?limit=0").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);

            // The address moved; the old one has no names left.
            let (_, gone) = get(&pool, "/v1/accounts/by-address/GALICE").await;
            assert_eq!(names(&gone).len(), 0);
            let (_, moved) = get(&pool, "/v1/accounts/by-address/GALICE2").await;
            assert_eq!(names(&moved), ["alicia"]);

            let (status, batch) = get(
                &pool,
                "/v1/accounts/by-address?addresses=GBOB,GALICE2,GNOBODY",
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(batch["result"]["GBOB"][0]["account_name"], "alice");
            assert_eq!(batch["result"]["GALICE2"][0]["flagged"], true);
            assert_eq!(batch["result"]["GNOBODY"], serde_json::json!([]));
            let (status, _) = get(&pool, "/v1/accounts/by-address").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);

            let pending: Vec<(String, String)> = sqlx::query_as(
                "SELECT account_name, address FROM v1.pending_account_preauths \
                 ORDER BY account_name",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(
                pending,
                [
                    ("carol".to_string(), "GCAROL".to_string()),
                    ("fay".to_string(), "GFAY".to_string()),
                ]
            );
        }
    }
}
