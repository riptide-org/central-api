use actix_web::{HttpRequest, web::{Data, Path}, HttpResponse, get};
use log::trace;
use rand::Rng;
use tokio::sync::mpsc;
use ws_com_framework::{FileId, Message};

use crate::{State, ServerId, db::{DbBackend, Database}};

/// Download a file from a client
// Dev note: This code is split off in a seperate handler function to allow testing
// via implementations/mocking.
async fn __download(_: HttpRequest, path: Path<(ServerId, FileId)>, state: Data<State>, db: impl DbBackend) -> HttpResponse {
    let (server_id, file_id) = path.into_inner();

    //Check server is online
    let reader = state.servers.read().await;
    let server_online = reader.contains_key(&server_id); //Duplicate req #cd
    if server_online {
        let (tx, mut rx) = mpsc::channel(100);

        let download_id = rand::thread_rng().gen();

        //Create a valid upload job
        state.requests.write().await.insert(download_id, tx);

        //Acquire channel to WS, and send upload req. to server
        let msg = format!("{}/upload/{}", state.base_url, download_id);
        let connected_servers = state.servers.read().await;
        let uploader_ws = connected_servers.get(&server_id).unwrap(); //Duplicate req #cd
        uploader_ws.send(Message::UploadTo(file_id, msg)).await.unwrap();

        let payload = async_stream::stream! {
            while let Some(v) = rx.recv().await {
                yield v;
            }
        };

        //create a streaming response
        HttpResponse::Ok()
            .content_type("text/html")
            .streaming(payload)
    } else {
        trace!("client attempted to request file {} from {:?}, but that server isn't connected", file_id, server_id);
        HttpResponse::NotFound()
            .content_type("text/html")
            .body("requested resource not found, the server may not be connected")
    }
}

/// Download a file from a client
#[get("/download/{server_id}/{file_id}")]
pub async fn download(req: HttpRequest, state: Data<State>, database: Data<Database>, path: Path<(ServerId, FileId)>) -> HttpResponse {
    __download(req, path, state, database).await
}


// #[cfg(test)]
// mod test {
//     use super::__download;

// }