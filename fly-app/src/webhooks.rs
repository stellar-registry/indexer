use actix_web::{
    body::BoxBody,
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    web, Error, HttpResponse,
};
use std::env;

const WEBHOOK_SECRET_ENV: &str = "INDEXER_WEBHOOK_SECRET";
const WEBHOOK_SECRET_HEADER: &str = "x-webhook-secret";

#[derive(Clone)]
pub(crate) struct WebhookConfig {
    secret: Option<String>,
    enabled: bool,
}

pub fn load_webhook_config() -> WebhookConfig {
    let is_fly = env::var("FLY_APP_NAME")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    let secret = env::var(WEBHOOK_SECRET_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty());

    if is_fly && secret.is_none() {
        panic!(
            "{WEBHOOK_SECRET_ENV} must be set when FLY_APP_NAME is present (webhook auth is required on Fly)"
        );
    }

    WebhookConfig {
        secret,
        enabled: is_fly,
    }
}

impl WebhookConfig {
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

pub async fn webhook_auth_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, Error> {
    let Some(cfg) = req.app_data::<web::Data<WebhookConfig>>() else {
        return Ok(req.into_response(HttpResponse::InternalServerError().finish()));
    };

    if !cfg.enabled {
        return next.call(req).await;
    }

    let provided = req
        .headers()
        .get(WEBHOOK_SECRET_HEADER)
        .and_then(|v| v.to_str().ok());

    let authorized = matches!(
        (provided, cfg.secret.as_deref()),
        (Some(provided), Some(secret)) if provided == secret
    );

    if !authorized {
        return Ok(req.into_response(HttpResponse::Unauthorized().finish()));
    }

    next.call(req).await
}
