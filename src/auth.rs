use actix_web::{post, HttpResponse};

/// Attempt to register a new webserver with the api
#[post("/register")]
pub async fn register() -> HttpResponse {
    todo!()
}