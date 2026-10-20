use actix_web::{
    body::{EitherBody, MessageBody},
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    web, Error, HttpMessage, HttpResponse,
};
use secrecy::{ExposeSecret, SecretString};
use std::env;
use subtle::ConstantTimeEq;

const API_KEYS_ENV: &str = "INDEXER_API_KEYS";
const API_KEY_HEADER: &str = "x-api-key";

#[derive(Clone, Default)]
pub(crate) struct ApiKeyConfig {
    // (consumer name, key)
    keys: Vec<(String, SecretString)>,
}

/// Marks a request as carrying a valid API key; the rate limiter skips these.
#[derive(Clone, Copy)]
pub(crate) struct ApiKeyAuthorized;

pub fn load_api_key_config() -> ApiKeyConfig {
    let raw = env::var(API_KEYS_ENV).unwrap_or_default();
    ApiKeyConfig::parse(&raw).unwrap_or_else(|e| panic!("{e}"))
}

impl ApiKeyConfig {
    /// Parses `name:key,name:key`.
    fn parse(raw: &str) -> Result<Self, String> {
        let mut keys = Vec::new();
        for (i, entry) in raw.split(',').map(str::trim).enumerate() {
            if entry.is_empty() {
                continue;
            }
            match entry.split_once(':').map(|(n, k)| (n.trim(), k.trim())) {
                Some((name, key)) if !name.is_empty() && !key.is_empty() => {
                    keys.push((name.to_string(), SecretString::from(key)))
                }
                // Report the position only; the entry itself may be a key.
                _ => return Err(format!("{API_KEYS_ENV} entry {i} must be `name:key`")),
            }
        }
        Ok(ApiKeyConfig { keys })
    }

    pub fn names(&self) -> Vec<&str> {
        self.keys.iter().map(|(name, _)| name.as_str()).collect()
    }

    // Checks every key without short-circuiting so timing doesn't reveal which one matched.
    fn consumer(&self, provided: &[u8]) -> Option<&str> {
        let mut matched = None;
        for (name, key) in &self.keys {
            if bool::from(key.expose_secret().as_bytes().ct_eq(provided)) {
                matched = Some(name.as_str());
            }
        }
        matched
    }
}

pub async fn api_key_middleware<B: MessageBody>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    let Some(provided) = req.headers().get(API_KEY_HEADER) else {
        return Ok(next.call(req).await?.map_into_left_body());
    };

    let consumer = req
        .app_data::<web::Data<ApiKeyConfig>>()
        .and_then(|cfg| cfg.consumer(provided.as_bytes()));

    let Some(consumer) = consumer else {
        return Ok(req
            .into_response(HttpResponse::Unauthorized().finish())
            .map_into_right_body());
    };

    ::tracing::info!(api_consumer = consumer, "api key request");

    req.extensions_mut().insert(ApiKeyAuthorized);
    Ok(next.call(req).await?.map_into_left_body())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limit;
    use actix_web::{http::StatusCode, middleware::from_fn, test as actix_test, App};

    #[test]
    fn parses_named_keys() {
        let cfg = ApiKeyConfig::parse(" a:key-a, ,b : key-b ,").unwrap();
        assert_eq!(cfg.names(), ["a", "b"]);
        assert_eq!(cfg.consumer(b"key-a"), Some("a"));
        assert_eq!(cfg.consumer(b"key-b"), Some("b"));
        assert_eq!(cfg.consumer(b"key-"), None);
        assert_eq!(cfg.consumer(b""), None);
        assert_eq!(ApiKeyConfig::parse("").unwrap().consumer(b""), None);
    }

    #[test]
    fn rejects_malformed_entries_without_echoing_them() {
        for raw in ["a:key-a,secret", "a:key-a,:secret", "a:"] {
            let err = ApiKeyConfig::parse(raw).err().unwrap();
            assert!(!err.contains("secret") && !err.contains("key-a"), "{err}");
        }
    }

    macro_rules! app {
        ($keys:expr, $burst:expr) => {
            actix_test::init_service(
                App::new()
                    .app_data(web::Data::new(ApiKeyConfig::parse($keys).unwrap()))
                    .service(
                        web::scope("/v1")
                            .wrap(rate_limit::governor(60, $burst))
                            .wrap(from_fn(api_key_middleware))
                            .route("", web::get().to(|| async { HttpResponse::Ok().finish() })),
                    ),
            )
            .await
        };
    }

    fn request(key: Option<&str>) -> actix_test::TestRequest {
        let req = actix_test::TestRequest::get()
            .uri("/v1")
            .insert_header(("Fly-Client-IP", "203.0.113.7"));
        match key {
            Some(key) => req.insert_header((API_KEY_HEADER, key)),
            None => req,
        }
    }

    #[actix_web::test]
    async fn rejects_unknown_key() {
        let app = app!("a:key-a", 5);
        let resp = actix_test::call_service(&app, request(Some("nope")).to_request()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn rejects_any_key_when_none_configured() {
        let app = app!("", 5);
        let resp = actix_test::call_service(&app, request(Some("key-a")).to_request()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn keyless_requests_are_rate_limited() {
        let app = app!("a:key-a", 1);
        let first = actix_test::call_service(&app, request(None).to_request()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let second = actix_test::call_service(&app, request(None).to_request()).await;
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[actix_web::test]
    async fn valid_keys_bypass_rate_limit() {
        let app = app!("a:key-a,b:key-b", 1);
        for key in ["key-a", "key-b", "key-a", "key-b"] {
            let resp = actix_test::call_service(&app, request(Some(key)).to_request()).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }
        // Keyed traffic doesn't use up the IP's allowance.
        let resp = actix_test::call_service(&app, request(None).to_request()).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
