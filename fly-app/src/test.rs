use crate::setup_routes;
use actix_web::dev::{ServiceFactory, ServiceRequest};
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

#[cfg(test)]
mod tests {
    use crate::test::test_get;

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
}
