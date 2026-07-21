use actix_governor::{
    governor::middleware::NoOpMiddleware, Governor, GovernorConfigBuilder, KeyExtractor,
    SimpleKeyExtractionError,
};
use actix_web::middleware::Condition;
use std::env;

const BURST_SIZE: u32 = 5;
const SEC_PER_REQ: u64 = 2;

#[derive(Clone, Default)]
pub struct Extractor {}

impl KeyExtractor for Extractor {
    type Key = String;
    type KeyExtractionError = SimpleKeyExtractionError<&'static str>;

    fn extract(
        &self,
        req: &actix_web::dev::ServiceRequest,
    ) -> Result<Self::Key, Self::KeyExtractionError> {
        let head = req.head();
        match head.headers().get("Fly-Client-IP") {
            Some(data) => return Ok(data.to_str().unwrap().to_string()),
            None => return Err(SimpleKeyExtractionError::new("can not find any token")),
        }
    }
}

pub fn middleware() -> Condition<Governor<Extractor, NoOpMiddleware>> {
    let is_fly = env::var("FLY_APP_NAME").is_ok();
    let governor_conf = GovernorConfigBuilder::default()
        .key_extractor(Extractor::default())
        .seconds_per_request(SEC_PER_REQ)
        .burst_size(BURST_SIZE)
        .finish()
        .expect("failed to initialize governor config");
    Condition::new(is_fly, Governor::new(&governor_conf))
}
