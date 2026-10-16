use actix_governor::{
    governor::middleware::NoOpMiddleware, Governor, GovernorConfigBuilder, KeyExtractor,
    SimpleKeyExtractionError,
};
use actix_web::{middleware::Condition, HttpMessage};
use std::env;

use crate::api_keys::ApiKeyAuthorized;

const BURST_SIZE: u32 = 5;
const SEC_PER_REQ: u64 = 2;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    Ip(String),
    ApiKey,
}

#[derive(Clone, Default)]
pub struct Extractor {}

impl KeyExtractor for Extractor {
    type Key = RateLimitKey;
    type KeyExtractionError = SimpleKeyExtractionError<&'static str>;

    fn extract(
        &self,
        req: &actix_web::dev::ServiceRequest,
    ) -> Result<Self::Key, Self::KeyExtractionError> {
        if req.extensions().contains::<ApiKeyAuthorized>() {
            return Ok(RateLimitKey::ApiKey);
        }
        let head = req.head();
        match head.headers().get("Fly-Client-IP") {
            Some(data) => return Ok(RateLimitKey::Ip(data.to_str().unwrap().to_string())),
            None => return Err(SimpleKeyExtractionError::new("can not find any token")),
        }
    }

    fn whitelisted_keys(&self) -> Vec<Self::Key> {
        vec![RateLimitKey::ApiKey]
    }
}

pub(crate) fn governor(
    seconds_per_request: u64,
    burst_size: u32,
) -> Governor<Extractor, NoOpMiddleware> {
    let governor_conf = GovernorConfigBuilder::default()
        .key_extractor(Extractor::default())
        .seconds_per_request(seconds_per_request)
        .burst_size(burst_size)
        .finish()
        .expect("failed to initialize governor config");
    Governor::new(&governor_conf)
}

pub fn middleware() -> Condition<Governor<Extractor, NoOpMiddleware>> {
    let is_fly = env::var("FLY_APP_NAME").is_ok();
    Condition::new(is_fly, governor(SEC_PER_REQ, BURST_SIZE))
}
