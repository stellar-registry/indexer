use actix_governor::{KeyExtractor, SimpleKeyExtractionError};

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
