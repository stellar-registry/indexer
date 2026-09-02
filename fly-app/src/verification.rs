use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::log_db_error;

#[derive(Serialize, Deserialize)]
pub struct VerifyBuildPayload {
    id: String,
    contract_id: String,
}

// Stellar Expert attests wasm builds via a GitHub Actions workflow that
// compiles a contract's source and publishes the resulting hash/repo/commit
// to their explorer — see https://github.com/stellar-expert/soroban-build-workflow.
// `path` is only present when the verified source lives in a subdirectory
// of the repository (a monorepo); Stellar Expert omits it for a repo-root
// build, so it's optional here too.
#[derive(Deserialize, Debug, Clone)]
struct StellarExpertValidation {
    status: String,
    repository: String,
    commit: String,
    package: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize)]
struct StellarExpertContractResponse {
    validation: Option<StellarExpertValidation>,
}

/// What we hand back to API callers once a contract has been checked and
/// found verified. Absent (`None`) covers both "not checked yet" and
/// "checked, not verified" — callers don't need to tell those apart.
#[derive(sqlx::FromRow, Serialize)]
pub struct VerificationInfo {
    status: String,
    repository: String,
    #[serde(rename = "commit")]
    commit_hash: String,
    package: String,
    path: Option<String>,
}

pub async fn fetch_contract_verification(
    pool: &PgPool,
    contract_id: &str,
) -> Option<VerificationInfo> {
    let row = sqlx::query_as::<_, VerificationInfo>(
        "SELECT status, repository, commit_hash, package, path \
         FROM v1.contract_verifications \
         WHERE contract_id = $1 AND status = 'verified'",
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await;

    match row {
        Ok(Some(v)) => Some(v),
        Ok(None) => None,
        Err(e) => {
            log_db_error("fetch_contract_verification", &e, pool);
            None
        }
    }
}

// "testnet" or "public" (Stellar Expert's own naming for mainnet) — see
// ui/app/lib/network.ts's stellarExpertNetworkSegment for the UI-side twin
// of this. Each network gets its own Fly app / env, so this is a plain env
// var rather than something threaded through a request.
fn stellar_expert_segment() -> &'static str {
    match std::env::var("STELLAR_EXPERT_SEGMENT").as_deref() {
        Ok("public") => "public",
        Ok("testnet") => "testnet",
        other => {
            ::tracing::warn!(
                value = ?other,
                "STELLAR_EXPERT_SEGMENT unset or invalid, defaulting to testnet"
            );
            "testnet"
        }
    }
}

async fn verify_contract(event_id: &str, contract_id: &str, pool: web::Data<PgPool>) {
    let already_checked = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM v1.contract_verifications WHERE contract_id = $1)",
    )
    .bind(contract_id)
    .fetch_one(pool.get_ref())
    .await;

    match already_checked {
        Ok(true) => {
            ::tracing::warn!(contract_id, "contract already checked for verification");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            log_db_error("verify_contract.already_checked", &e, pool.get_ref());
            return;
        }
    }

    let segment = stellar_expert_segment();
    let url = format!("https://api.stellar.expert/explorer/{segment}/contract/{contract_id}");

    let response = match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => resp,
        Ok(resp) => {
            ::tracing::warn!(
                contract_id,
                status = %resp.status(),
                "stellar expert returned a non-success status"
            );
            return;
        }
        Err(e) => {
            ::tracing::warn!(contract_id, error = %e, "failed to reach stellar expert");
            return;
        }
    };

    let validation = match response.json::<StellarExpertContractResponse>().await {
        Ok(body) => body.validation,
        Err(e) => {
            ::tracing::warn!(
                contract_id,
                error = %e,
                "failed to parse stellar expert response"
            );
            return;
        }
    };

    // There's no distinct "unverified" status to check for — the
    // `validation` key is simply absent from the response otherwise. We
    // still write a row either way (status NULL when not verified), so
    // the idempotency check above skips this contract on any future
    // replay/retry instead of re-fetching forever.
    let verified = validation.filter(|v| v.status == "verified");

    let ledger_sequence = match sqlx::query_scalar::<_, i64>(
        "SELECT ledger_sequence FROM v1.registered_contracts WHERE id = $1",
    )
    .bind(event_id)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(seq) => seq,
        Err(e) => {
            log_db_error("verify_contract.select_ledger_sequence", &e, pool.get_ref());
            None
        }
    };

    // `verified` isn't read after this, so consume it by value and
    // destructure once instead of re-borrowing per field.
    let (status, repository, commit, package, path) = match verified {
        Some(v) => (
            Some(v.status),
            Some(v.repository),
            Some(v.commit),
            Some(v.package),
            v.path,
        ),
        None => (None, None, None, None, None),
    };

    let insert_result = sqlx::query(
        "INSERT INTO v1.contract_verifications \
            (contract_id, status, repository, commit_hash, package, path, checked_at, ledger_sequence) \
         VALUES ($1, $2, $3, $4, $5, $6, now(), $7) \
         ON CONFLICT (contract_id) DO NOTHING",
    )
    .bind(contract_id)
    .bind(status)
    .bind(repository)
    .bind(commit)
    .bind(package)
    .bind(path)
    .bind(ledger_sequence)
    .execute(pool.get_ref())
    .await;

    match insert_result {
        Ok(result) => {
            if result.rows_affected() == 0 {
                ::tracing::warn!(
                    contract_id,
                    "verification insertion skipped because row already exists"
                );
            }
        }
        Err(e) => {
            log_db_error(
                "verify_contract.insert_contract_verifications",
                &e,
                pool.get_ref(),
            );
        }
    }
}

pub async fn verify_build_webhook(
    payload: web::Json<VerifyBuildPayload>,
    pool: web::Data<PgPool>,
) -> HttpResponse {
    // spawn requires it to be 'static because it might outlive the task, cloning it will make the
    // spawned task own a PgPool.
    let pool = pool.clone();
    let _handle = actix_web::rt::spawn(async move {
        verify_contract(payload.id.as_str(), payload.contract_id.as_str(), pool).await;
    });

    HttpResponse::Ok().finish()
}
