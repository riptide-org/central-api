use actix_web::{post, web::Data, HttpResponse};
use log::error;
use rand::{distributions::Alphanumeric, Rng};

use crate::{
    db::{Database, DbBackend, DbBackendError},
    error::HttpError,
};

/// Attempt to register a new webserver with the api
async fn __register(state: impl DbBackend) -> Result<HttpResponse, HttpError> {
    //Generate a public id for this api
    let mut rand = rand::thread_rng();
    let mut id: u64 = rand.gen();
    while state.contains_entry(&id).await? {
        id = rand.gen();
    }

    //Generate a passcode
    let passcode: Vec<u8> = rand.sample_iter(&Alphanumeric).take(32).collect();

    //Insert passcode into database, if an error occurs
    if let Some(prev) = state
        .save_entry(
            id,
            passcode.clone(), /* Bad Clone, could be ampersand */
        )
        .await?
    {
        state.save_entry(id, prev).await?; //return old code to previous state
        error!("Unlikely error occured: server_id was duplicate");
        return Err(DbBackendError::AlreadyExist.into());
    }

    //Return this new public_id/passcode pair
    Ok(HttpResponse::Created()
        .content_type("application/json")
        .body(format!(
            "{{\"public_id\":{},\"passcode\":\"{}\"}}",
            id,
            unsafe { std::str::from_utf8_unchecked(&passcode) }
        )))
}

#[post("/register")]
pub async fn register(state: Data<Database>) -> impl actix_web::Responder {
    __register(state).await
}

#[cfg(test)]
mod test {
    use actix_web::body::MessageBody;
    use serde::Deserialize;
    use std::sync::Arc;

    use super::__register;
    use crate::db::{Database, DbBackend, MockDb};

    #[derive(Debug, Deserialize)]
    struct Id {
        public_id: u64,
        passcode: String,
    }

    #[tokio::test]
    async fn test_registering_api() {
        const CONVERSION_COUNT: u64 = 100_000;
        let db = Arc::new(MockDb::new().await.unwrap());

        let mut keys = Vec::with_capacity(CONVERSION_COUNT as usize);

        for _ in 0..CONVERSION_COUNT {
            let i = __register(db.clone()).await.expect("registered properly");
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

    //XXX: on failure remove the db file (e.g. ON DROP)
    #[tokio::test]
    async fn test_registering_api_real_db() {
        const CONVERSION_COUNT: u64 = 100;

        std::fs::write("./test-db.db", b"").expect("able to write to db");
        let tmp_var = std::env::var("DATABASE_URL").unwrap_or_else(|_| String::from(""));
        std::env::set_var("DATABASE_URL", "./test-db.db");

        let db = Arc::new(Database::new().await.unwrap());

        db.init().await.expect("valid db");

        let mut keys = Vec::with_capacity(CONVERSION_COUNT as usize);

        for _ in 0..CONVERSION_COUNT {
            let i = __register(db.clone()).await.expect("registered properly");
            //parse from body
            let body_bytes = i.into_body().try_into_bytes().unwrap();
            let body = std::str::from_utf8(&body_bytes).expect("valid string");
            let id: Id = serde_json::from_str(body).expect("unabel to parse from json");
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

        std::env::set_var("DATABASE_URL", tmp_var);
        std::fs::remove_file("./test-db.db").expect("able to write to db");
    }
}
