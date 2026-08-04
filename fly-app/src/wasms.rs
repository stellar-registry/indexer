use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx;
use sqlx::{types::Json, PgPool};
use stellar_xdr::curr::{ScMetaEntry, ScMetaV0, ScSpecEntry, ScSpecTypeDef};

use crate::log_db_error;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FunctionInput {
    doc: String,
    name: String,
    #[serde(rename = "type")]
    type_: ScSpecTypeDef,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FunctionSpec {
    doc: Option<String>,        // None if empty
    inputs: Vec<FunctionInput>, // (arg_name, arg_type)
}

#[derive(Serialize, Deserialize)]
pub struct WasmDetailPayload {
    id: String,
    wasm_hash: String,
}

#[derive(sqlx::FromRow)]
struct ExtractedWasmRow {
    contract_meta: Option<Json<WasmMeta>>,
    contract_spec: Option<Json<serde_json::Map<String, serde_json::Value>>>,
}

#[derive(Serialize, Deserialize)]
pub struct WasmMeta {
    rsver: Option<String>,
    rssdkver: Option<String>,
    rssdk_spec_shaking: Option<String>,
    cliver: Option<String>,
    source_repo: Option<String>,
    binver: Option<String>,
}

pub async fn fetch_wasm_meta(pool: &PgPool, wasm_hash: &str) -> Option<WasmMeta> {
    let wasm_meta = sqlx::query_as::<_, ExtractedWasmRow>(
        "SELECT NULL AS contract_spec, contract_meta FROM v1.extracted_wasm_details WHERE wasm_hash = $1",
    )
    .bind(wasm_hash)
    .fetch_optional(pool)
    .await;
    match wasm_meta {
        Ok(Some(row)) => row.contract_meta.map(|data| data.0),
        Ok(None) => {
            ::tracing::warn!(wasm_hash, "no wasm binary found");
            return None;
        }
        Err(e) => {
            log_db_error("fetch_wasm_meta", &e, pool);
            return None;
        }
    }
}

pub async fn fetch_wasm_spec(
    pool: &PgPool,
    wasm_hash: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, sqlx::Error> {
    let wasm_spec = sqlx::query_as::<_, ExtractedWasmRow>(
        "SELECT contract_spec, NULL as contract_meta FROM v1.extracted_wasm_details WHERE wasm_hash = $1",
    )
    .bind(wasm_hash)
    .fetch_optional(pool)
    .await;

    match wasm_spec {
        Ok(Some(row)) => Ok(row.contract_spec.map(|data| data.0)),
        Ok(None) => {
            ::tracing::warn!(wasm_hash, "no wasm binary found");
            return Ok(None);
        }
        Err(e) => {
            log_db_error("fetch_wasm_spec", &e, pool);
            return Err(e);
        }
    }
}

fn parse_wasm_meta(
    wasm: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, soroban_meta::read::FromWasmError> {
    let meta = soroban_meta::read::from_wasm(&wasm);
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

            return Ok(obj);
        }
        Err(e) => Err(e),
    }
}

fn parse_wasm_spec(
    wasm: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, soroban_spec::read::FromWasmError> {
    let spec = soroban_spec::read::from_wasm(&wasm);
    match spec {
        Ok(entries) => {
            let mut obj = serde_json::Map::new();
            let constructor = entries.into_iter().find_map(|entry| match entry {
                ScSpecEntry::FunctionV0(f) if f.name.to_utf8_string_lossy() == "__constructor" => {
                    Some(f)
                }
                _ => None,
            });
            constructor.map(|f| {
                let spec = FunctionSpec {
                    doc: Some(f.doc.to_utf8_string_lossy()),
                    inputs: f
                        .inputs
                        .iter()
                        .map(|i| FunctionInput {
                            doc: i.doc.to_utf8_string_lossy(),
                            name: i.name.to_utf8_string_lossy(),
                            type_: i.type_.clone(),
                        })
                        .collect(),
                };
                match serde_json::to_value(spec) {
                    Ok(spec_json) => {
                        obj.insert("__constructor".to_string(), spec_json);
                    }
                    Err(e) => {
                        ::tracing::warn!(
                            error = %e,
                            error_debug = ?e,
                            "failed to serialize constructor spec"
                        );
                    }
                }
            });
            return Ok(obj);
        }
        Err(e) => Err(e),
    }
}

async fn extract_wasm_details(wasm_hash: &str, pool: web::Data<PgPool>) {
    let is_extracted = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM v1.extracted_wasm_details WHERE wasm_hash = $1)",
    )
    .bind(wasm_hash)
    .fetch_one(pool.get_ref())
    .await;

    match is_extracted {
        Ok(true) => {
            ::tracing::warn!(wasm_hash, "wasm details already extracted");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            log_db_error("extract_wasm_details.exists", &e, pool.get_ref());
            return;
        }
    }

    let wasm_record = sqlx::query_as::<_, (Vec<u8>, i64)>(
        "SELECT decode(wasm, 'hex') AS wasm_binary, ledger_sequence \
         FROM archive.wasm_binaries \
         WHERE wasm_hash = $1",
    )
    .bind(wasm_hash)
    .fetch_optional(pool.get_ref())
    .await;

    let (wasm_binary, ledger_sequence) = match wasm_record {
        Ok(Some(record)) => record,
        Ok(None) => {
            ::tracing::warn!(wasm_hash, "wasm hash not found in archive.wasm_binaries");
            return;
        }
        Err(e) => {
            log_db_error("extract_wasm_details.wasm_binary", &e, pool.get_ref());
            return;
        }
    };

    let contract_meta = match parse_wasm_meta(&wasm_binary) {
        Ok(meta) => {
            if let Err(e) =
                serde_json::from_value::<WasmMeta>(serde_json::Value::Object(meta.clone()))
            {
                ::tracing::warn!(
                    wasm_hash,
                    error = %e,
                    error_debug = ?e,
                    "failed to validate wasm metadata against WasmMeta schema"
                );
            }

            Some(serde_json::Value::Object(meta).to_string())
        }
        Err(e) => {
            ::tracing::warn!(
                wasm_hash,
                error = %e,
                error_debug = ?e,
                "failed to parse wasm metadata"
            );
            None
        }
    };

    let contract_spec = match parse_wasm_spec(&wasm_binary) {
        Ok(spec) => Some(serde_json::Value::Object(spec).to_string()),
        Err(e) => {
            ::tracing::warn!(
                wasm_hash,
                error = %e,
                error_debug = ?e,
                "failed to parse wasm constructor spec"
            );
            None
        }
    };

    let insert_result = sqlx::query(
        "INSERT INTO v1.extracted_wasm_details (wasm_hash, contract_spec, contract_meta, ledger_sequence) \
         VALUES ($1, $2::jsonb, $3::jsonb, $4) \
         ON CONFLICT (wasm_hash) DO NOTHING",
    )
    .bind(wasm_hash)
    .bind(contract_spec.as_deref())
    .bind(contract_meta.as_deref())
    .bind(ledger_sequence)
    .execute(pool.get_ref())
    .await;

    match insert_result {
        Ok(result) => {
            if result.rows_affected() == 0 {
                ::tracing::warn!(
                    wasm_hash,
                    "wasm details insertion skipped because row already exists"
                );
            }
        }
        Err(e) => {
            log_db_error(
                "extract_wasm_details.insert_extracted_wasm_details",
                &e,
                pool.get_ref(),
            );
        }
    }
}

pub async fn wasm_details_webhook(
    payload: actix_web::web::Json<WasmDetailPayload>,
    pool: web::Data<PgPool>,
) -> HttpResponse {
    // spawn requires it to be 'static because it might outlive the task, cloning it will make the
    // spawned task own a PgPool.
    let pool = pool.clone();
    let _handle = actix_web::rt::spawn(async move {
        extract_wasm_details(payload.wasm_hash.as_str(), pool).await;
    });

    HttpResponse::Ok().finish()
}
