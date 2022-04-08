use actix_web::{HttpRequest, web::{Payload, Data, Path}, HttpResponse, post};
use futures::StreamExt;
use log::error;

use crate::{State, ServerId};


/// Internal upload handler function for a waiting client
// Dev note: This code is split off in a seperate handler function to allow testing
// via implementations/mocking.
async fn __upload(_: HttpRequest, mut payload: Payload, state: Data<State>, path: Path<ServerId>) -> HttpResponse {
    let upload_id = path.into_inner();

    //Get uploadee channel
    let mut sender_store = state.requests.write().await;
    println!("the upload id is: {upload_id:?}");
    println!("available keys are: {:?}", sender_store.keys());
    let sender = sender_store.remove(&upload_id).unwrap();

    //XXX timeout?
    while let Some(chk) = payload.next().await {
        if let Err(e) = sender.send(chk).await {
            error!("problem sending payload {:?}", e);
            return HttpResponse::InternalServerError()
                .body("upload failed");
        };
    };

    HttpResponse::Ok()
        .body("succesfully uploaded")
}

/// Upload a file or metadata to a waiting client
#[post("/upload/{upload_id}")]
pub async fn upload(req: HttpRequest, payload: Payload, state: Data<State>, path: Path<ServerId>) -> HttpResponse {
    __upload(req, payload, state, path).await
}