use crate::setup_routes;
use actix_web::dev::{ServiceFactory, ServiceRequest};
use actix_web::http::StatusCode;
use actix_web::web::Data;
use actix_web::{test, App, Error};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;

async fn setup_test<T>(app: App<T>) -> App<T>
where
    T: ServiceFactory<ServiceRequest, Config = (), Error = Error, InitError = ()>,
{
    let database_url = std::env::var("DATABASE_URL").unwrap_or(String::from("postgresql://postgres:postgres@localhost:5432/postgres"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    setup_routes(app.app_data(Data::new(pool.clone())))
}

pub async fn test_get(path: &str, expected_json: &str) {
    let app = test::init_service(setup_test(App::new()).await).await;
    let req = test::TestRequest::get().uri(path).to_request();
    let resp = String::from_utf8(Vec::from(test::call_and_read_body(&app, req).await)).unwrap();
    println!("received response: {:?}", &resp);
    let expected: Value = serde_json::from_str(expected_json).unwrap();
    let actual: Value = serde_json::from_str(resp.as_str()).unwrap();
    assert_eq!(expected, actual);
}

pub async fn test_get_status(path: &str, expected_status: StatusCode, expected_json: &str) {
    let app = test::init_service(setup_test(App::new()).await).await;
    let req = test::TestRequest::get().uri(path).to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), expected_status);
    let body = test::read_body(resp).await;
    let actual: Value = serde_json::from_slice(&body).unwrap();
    let expected: Value = serde_json::from_str(expected_json).unwrap();
    assert_eq!(expected, actual);
}

#[cfg(test)]
mod tests {
    use crate::test::{test_get, test_get_status};
    use actix_web::http::StatusCode;

    #[actix_web::test]
    async fn test_index_get() {
        test_get(
            "/",
            r#"
        {
         "name": "Registry Indexer API",
         "versions": [
           {
             "path": "/v1",
             "status": "current",
             "version": "v1"
           }
         ]
        }
        "#,
        )
        .await;
    }

    // Channel-validator negative paths. These guard against a regression where
    // the error message copied from the limit validator ("Limit must be an
    // integer between 2 and 200") was served on invalid channels.
    const BAD_CHANNEL_JSON: &str = r#"{"error": "Channel must be one of: main, unverified"}"#;

    #[actix_web::test]
    async fn test_wasm_latest_bad_channel() {
        test_get_status(
            "/v1/wasms/bogus/registry",
            StatusCode::BAD_REQUEST,
            BAD_CHANNEL_JSON,
        )
        .await;
    }

    #[actix_web::test]
    async fn test_wasm_version_bad_channel() {
        test_get_status(
            "/v1/wasms/bogus/registry/v/0.4.1",
            StatusCode::BAD_REQUEST,
            BAD_CHANNEL_JSON,
        )
        .await;
    }

    #[actix_web::test]
    async fn test_single_contract_bad_channel() {
        test_get_status(
            "/v1/contracts/bogus/registry",
            StatusCode::BAD_REQUEST,
            BAD_CHANNEL_JSON,
        )
        .await;
    }

    #[actix_web::test]
    async fn test_contract_deploy_detail_bad_channel() {
        test_get_status(
            "/v1/contract_deploy_details/bogus/registry",
            StatusCode::BAD_REQUEST,
            BAD_CHANNEL_JSON,
        )
        .await;
    }
}
