use actix_web::{
    body::{EitherBody, MessageBody},
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    web, Error, HttpMessage, HttpResponse,
};
use std::env;
use subtle::ConstantTimeEq;

const API_KEYS_ENV: &str = "INDEXER_API_KEYS";
const API_KEY_HEADER: &str = "x-api-key";

// No Debug derive so keys can't end up in logs.
#[derive(Clone, Default)]
pub(crate) struct ApiKeyConfig {
    keys: Vec<String>,
}

/// Marks a request as carrying a valid API key; the rate limiter skips these.
#[derive(Clone, Copy)]
pub(crate) struct ApiKeyAuthorized;

pub fn load_api_key_config() -> ApiKeyConfig {
    let raw = env::var(API_KEYS_ENV).unwrap_or_default();
    ApiKeyConfig::from_csv(&raw)
}

impl ApiKeyConfig {
    fn from_csv(raw: &str) -> Self {
        let keys = raw
            .split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(String::from)
            .collect();
        ApiKeyConfig { keys }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    // Checks every key without short-circuiting so timing doesn't reveal which one matched.
    fn is_valid(&self, provided: &[u8]) -> bool {
        self.keys.iter().fold(0u8, |acc, key| {
            acc | key.as_bytes().ct_eq(provided).unwrap_u8()
        }) == 1
    }
}

pub async fn api_key_middleware<B: MessageBody>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    let Some(provided) = req.headers().get(API_KEY_HEADER) else {
        return Ok(next.call(req).await?.map_into_left_body());
    };

    let authorized = req
        .app_data::<web::Data<ApiKeyConfig>>()
        .is_some_and(|cfg| cfg.is_valid(provided.as_bytes()));

    if !authorized {
        return Ok(req
            .into_response(HttpResponse::Unauthorized().finish())
            .map_into_right_body());
    }

    req.extensions_mut().insert(ApiKeyAuthorized);
    Ok(next.call(req).await?.map_into_left_body())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limit;
    use actix_web::{http::StatusCode, middleware::from_fn, test as actix_test, App};

    #[test]
    fn parses_comma_separated_keys() {
        let cfg = ApiKeyConfig::from_csv(" key-a, ,key-b ,");
        assert_eq!(cfg.len(), 2);
        assert!(cfg.is_valid(b"key-a"));
        assert!(cfg.is_valid(b"key-b"));
        assert!(!cfg.is_valid(b"key-"));
        assert!(!cfg.is_valid(b""));
        assert!(!ApiKeyConfig::from_csv("").is_valid(b""));
    }

    macro_rules! app {
        ($keys:expr, $burst:expr) => {
            actix_test::init_service(
                App::new()
                    .app_data(web::Data::new(ApiKeyConfig::from_csv($keys)))
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
        let app = app!("key-a", 5);
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
        let app = app!("key-a", 1);
        let first = actix_test::call_service(&app, request(None).to_request()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let second = actix_test::call_service(&app, request(None).to_request()).await;
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[actix_web::test]
    async fn valid_keys_bypass_rate_limit() {
        let app = app!("key-a,key-b", 1);
        for key in ["key-a", "key-b", "key-a", "key-b"] {
            let resp = actix_test::call_service(&app, request(Some(key)).to_request()).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }
        // Keyed traffic doesn't use up the IP's allowance.
        let resp = actix_test::call_service(&app, request(None).to_request()).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
