use actix_web::HttpResponse;
use serde::Serialize;
use sqlx::PgPool;
use tracing_actix_web::RequestId;

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize)]
pub struct InternalErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub fn internal_server_error_response(request_id: RequestId) -> HttpResponse {
    HttpResponse::InternalServerError().json(InternalErrorResponse {
        error: "Internal server error".into(),
        request_id: Some(request_id.to_string()),
    })
}

pub fn log_db_error(operation: &'static str, error: &sqlx::Error, pool: &PgPool) {
    ::tracing::error!(
        operation,
        error = %error,
        error_debug = ?error,
        pool_size = pool.size(),
        pool_idle = pool.num_idle(),
        "database query failed"
    );
}
