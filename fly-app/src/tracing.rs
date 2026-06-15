use tracing_actix_web::{DefaultRootSpanBuilder, RootSpanBuilder};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

pub struct RootSpan;

impl RootSpanBuilder for RootSpan {
    fn on_request_start(request: &actix_web::dev::ServiceRequest) -> tracing::Span {
        let req_id: Uuid = Uuid::new_v4();
        let req_id = req_id.to_string();
        tracing_actix_web::root_span!(request, req_id)
    }

    fn on_request_end<B: actix_web::body::MessageBody>(
        span: tracing::Span,
        outcome: &Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>,
    ) {
        DefaultRootSpanBuilder::on_request_end(span, outcome);
    }
}

pub fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,actix_web=info,sqlx=warn".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(true),
        )
        .init();
}
