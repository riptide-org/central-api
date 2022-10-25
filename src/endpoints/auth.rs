//! Handles authentication and registration of agents to the api

use actix_web::{post, web::Data, HttpRequest, HttpResponse};
use log::error;
use rand::{distributions::Alphanumeric, Rng};

use crate::{
    config::Config,
    db::{Database, DbBackend},
};

/// Attempt to register a new webserver with the api
async fn __register(
    req: HttpRequest,
    state: impl DbBackend,
    config: Data<Config>,
) -> Result<HttpResponse, HttpResponse> {
    if let Some(psk) = &config.password {
        let header = req.headers().get("Authorization");
        if let Some(header) = header {
            let header = header
                .to_str()
                .map_err(|_| HttpResponse::BadRequest().finish())?;
            if header != format!("Basic {}", psk) {
                return Err(HttpResponse::Unauthorized().finish());
            }
        } else {
            return Err(HttpResponse::Unauthorized().finish());
        }
    }

    // generate a public id for this api
    let mut rand = rand::thread_rng();
    let mut id: u64 = rand.gen();
    loop {
        match state.contains_entry(&id).await {
            Ok(true) => id = rand.gen(),
            Ok(false) => break,
            Err(e) => {
                error!("error while checking if id exists: {}", e);
                return Err(HttpResponse::InternalServerError().finish());
            }
        }
    }

    // generate a passcode
    let passcode: Vec<u8> = rand.sample_iter(&Alphanumeric).take(32).collect();

    // insert passcode into database, if an error occurs
    if let Some(prev) = state
        .save_entry(
            id,
            passcode.clone(), /* Bad Clone, could be ampersand */
        )
        .await
        .map_err(|e| {
            error!("error while saving entry: {}", e);
            HttpResponse::InternalServerError().finish()
        })?
    {
        state.save_entry(id, prev).await.map_err(|e| {
            error!("error while saving entry: {}", e);
            HttpResponse::InternalServerError().finish()
        })?;
        error!("Unlikely error occurred: server_id was duplicate");
        return Err(HttpResponse::InternalServerError().finish());
    }

    // return this new public_id/passcode pair
    Ok(HttpResponse::Created()
        .content_type("application/json")
        .body(format!(
            "{{\"public_id\":{},\"passcode\":\"{}\"}}",
            id,
            std::str::from_utf8(&passcode).expect("valid utf")
        )))
}

/// Endpoint for registering a new agent with the api (POST /register)
#[post("/register")]
pub async fn register(
    req: HttpRequest,
    state: Data<Database>,
    config: Data<Config>,
) -> HttpResponse {
    match __register(req, state, config).await {
        Ok(resp) => resp,
        Err(resp) => resp,
    }
}

#[cfg(test)]
#[cfg(not(tarpaulin_include))]
mod test {
    use actix_web::body::MessageBody;
    use actix_web::web::Data;
    use serde::Deserialize;
    use std::net::TcpListener;
    use std::sync::Arc;

    use super::__register;
    use crate::config::Config;
    use crate::db::tests::MockDb;
    use crate::db::{Database, DbBackend};

    fn find_open_port() -> TcpListener {
        for port in 1025..65535 {
            if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
                return l;
            }
        }
        panic!("no open ports found");
    }

    #[derive(Debug, Deserialize)]
    struct Id {
        public_id: u64,
        passcode: String,
    }

    #[tokio::test]
    async fn test_registering_api() {
        const CONVERSION_COUNT: u64 = 100_000;
        let db = Arc::new(MockDb::new(String::new()).await.unwrap());

        let mut keys = Vec::with_capacity(CONVERSION_COUNT as usize);

        let config: Config = Config {
            listener: find_open_port(),
            db_url: String::new(),
            base_url: "https://localhost:8080".into(),
            auth_timeout_seconds: 3,
            request_timeout_seconds: 3,
            ping_interval: 3,
            password: None,
        };
        let config = Data::new(config);

        let req = actix_web::test::TestRequest::default().to_http_request();

        for _ in 0..CONVERSION_COUNT {
            let i = __register(req.clone(), db.clone(), config.clone())
                .await
                .expect("registered properly");
            //parse from body
            let body_bytes = i.into_body().try_into_bytes().unwrap();
            let body = std::str::from_utf8(&body_bytes).expect("valid string");
            let id: Id = serde_json::from_str(body).expect("unabel to parse from json");
            keys.push(id);
        }

        for i in 0..CONVERSION_COUNT {
            let id = &keys[i as usize];
            assert!(db
                .validate_server(&id.public_id, &id.passcode.as_bytes().to_vec())
                .await
                .expect("able to login"));
        }
    }

    #[tokio::test]
    async fn test_registering_api_real_db() {
        const CONVERSION_COUNT: u64 = 100;

        std::fs::write("./test-db.db", b"").expect("able to write to db");

        let db = Arc::new(Database::new(String::from("./test-db.db")).await.unwrap());

        db.init().await.expect("valid db");

        let mut keys = Vec::with_capacity(CONVERSION_COUNT as usize);

        let config: Config = Config {
            listener: find_open_port(),
            db_url: String::new(),
            base_url: "https://localhost:8080".into(),
            auth_timeout_seconds: 3,
            request_timeout_seconds: 3,
            ping_interval: 3,
            password: None,
        };
        let config = Data::new(config);

        let req = actix_web::test::TestRequest::default().to_http_request();

        for _ in 0..CONVERSION_COUNT {
            let i = __register(req.clone(), db.clone(), config.clone())
                .await
                .expect("registered properly");
            //parse from body
            let body_bytes = i.into_body().try_into_bytes().unwrap();
            let body = std::str::from_utf8(&body_bytes).expect("valid string");
            let id: Id = serde_json::from_str(body).expect("unable to parse from json");
            keys.push(id);
        }

        for i in 0..CONVERSION_COUNT {
            let id = &keys[i as usize];
            let result = db
                .validate_server(&id.public_id, &id.passcode.as_bytes().to_vec())
                .await
                .expect("able to login");
            assert!(result);
        }

        std::fs::remove_file("./test-db.db").expect("able to write to db");
    }

    #[tokio::test]
    async fn test_should_fail_unauthorized() {
        let db = Arc::new(MockDb::new(String::new()).await.unwrap());

        let config: Config = Config {
            listener: find_open_port(),
            db_url: String::new(),
            base_url: "https://localhost:8080".into(),
            auth_timeout_seconds: 3,
            request_timeout_seconds: 3,
            ping_interval: 3,
            password: Some("password".into()),
        };
        let config = Data::new(config);

        // no password
        let req = actix_web::test::TestRequest::default().to_http_request();
        let i = __register(req, db.clone(), config.clone())
            .await
            .expect_err("registered properly");
        assert_eq!(i.status(), actix_web::http::StatusCode::UNAUTHORIZED);

        // invalid password
        let req = actix_web::test::TestRequest::default()
            .insert_header(("Authorization", "Basic invalid"))
            .to_http_request();
        let i = __register(req, db.clone(), config.clone())
            .await
            .expect_err("registered properly");
        assert_eq!(i.status(), actix_web::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_should_succeed_authorized() {
        let db = Arc::new(MockDb::new(String::new()).await.unwrap());

        let config: Config = Config {
            listener: find_open_port(),
            db_url: String::new(),
            base_url: "https://localhost:8080".into(),
            auth_timeout_seconds: 3,
            request_timeout_seconds: 3,
            ping_interval: 3,
            password: Some("password".into()),
        };
        let config = Data::new(config);

        // valid password
        let req = actix_web::test::TestRequest::default()
            .insert_header(("Authorization", "Basic password"))
            .to_http_request();
        let i = __register(req, db.clone(), config.clone())
            .await
            .expect("registered properly");
        assert_eq!(i.status(), actix_web::http::StatusCode::CREATED);
    }
}
