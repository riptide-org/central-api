use actix_web::{post, HttpResponse, web::Data};
use log::error;
use rand::Rng;
use ws_com_framework::{PublicId, Passcode};

use crate::{db::{Database, DbBackend, DbBackendError}, error::HttpError};

pub async fn validate_login(server_id: PublicId, passcode: Passcode, state: impl DbBackend) -> Result<bool, HttpError> {
    todo!()
}

/// Attempt to register a new webserver with the api
async fn __register(state: impl DbBackend) -> Result<HttpResponse, HttpError> {
    //Generate a public id for this api
    let mut rand = rand::thread_rng();
    let mut id: u64 = rand.gen();
    while state.contains_entry(&id).await? {
        id = rand.gen();
    }

    //Generate a passcode
    let mut passcode = Vec::with_capacity(32);
    for _ in 0..32 {
        passcode.push(rand.gen_range(b'A'..b'Z') as u8)
    }

    //Insert passcode into database, if an error occurs
    if let Some(prev) = state.save_entry(id, passcode.clone() /* Bad Clone, could be ampersand */).await? {
        state.save_entry(id, prev).await?; //return old code to previous state
        error!("Unlikely error occured: server_id was duplicate");
        return Err(DbBackendError::AlreadyExist.into())
    }

    //Return this new public_id/passcode pair
    Ok(
        HttpResponse::Created()
            .body(format!("{{
                \"public_id\":\"{}\",
                \"passcode\":\"{}\"
            }}", id, unsafe { std::str::from_utf8_unchecked(&passcode) }))
    )
}

#[post("/register")]
pub async fn register(state: Data<Database>) -> Result<HttpResponse, HttpError> {
    __register(state).await
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::db::{MockDb, DbBackend};
    use super::__register;

    #[tokio::test]
    async fn test_registering_api() {
        const CONVERSION_COUNT: u64 = 1_000_000;
        let db = Arc::new(MockDb::new().await.unwrap());

        for i in 0..CONVERSION_COUNT {
            let item = __register(db.clone()).await.expect("registered properly");
        }

        //XXX: validate output here?
    }
}