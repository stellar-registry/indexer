use sqlx::PgPool;
use stellar_xdr::curr::{ScMetaEntry, ScMetaV0};

fn parse_wasm_meta(wasm: &[u8]) -> Result<serde_json::Map, soroban_meta::read::FromWasmError> {
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

fn parse_wasm_spec(wasm: &[u8]) -> Result<serde_json::Map, soroban_spec::read::FromWasmError> {
    let spec = soroban_spec::read::from_wasm(&wasm);
    match spec {
        Ok(entries) => {
            let mut obj = serde_json::Map::new();
            for entry in entries {}
            return Ok(obj);
        }
        Err(e) => Err(e),
    }
}

pub async fn extract_wasm_details(pool: &PgPool, wasm_hash: &str) {
    let wasm_bytes = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT decode(wasm, 'hex') FROM archive.wasm_binaries WHERE wasm_hash = $1",
    )
    .bind(wasm_hash)
    .fetch_optional(pool)
    .await;

    match wasm_bytes {
        Ok(Some(bytes)) => {
            let metadata = parse_wasm_meta(&bytes);
        }
        Ok(None) => {}
        Err(e) => {}
    }
}
