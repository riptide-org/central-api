use actix_web::{
    get,
    web::{Data, Path},
    HttpRequest, HttpResponse,
};
use log::trace;
use rand::Rng;
use tokio::sync::mpsc;
use ws_com_framework::{FileId, Message};

use crate::{ServerId, State};

async fn __metadata(
    _: HttpRequest,
    path: Path<(ServerId, FileId)>,
    state: Data<State>,
) -> HttpResponse {
    let (server_id, file_id) = path.into_inner();
    trace!("metadata request recieved for {}, {}", server_id, file_id);

    //Check server is online
    let reader = state.servers.read().await;
    let server_online = reader.contains_key(&server_id); //Duplicate req #cd
    trace!("checking if server `{:?}` is online", &server_id);
    if server_online {
        trace!("server `{:?}` is online", &server_id);
        let (tx, mut rx) = mpsc::channel(100);

        let download_id = rand::thread_rng().gen();

        trace!("download id is {:?}", download_id);

        //Create a valid upload job
        state.requests.write().await.insert(download_id, tx);

        //Acquire channel to WS, and send upload req. to server
        trace!("uploading req to server");
        let connected_servers = state.servers.read().await;
        let uploader_ws = connected_servers.get(&server_id).unwrap(); //Duplicate req #cd
        uploader_ws
            .send(Message::MetadataReq(file_id, download_id))
            .await
            .unwrap();

        trace!("generating response payload and returning");

        let payload = async_stream::stream! {
            //XXX: enable timeouts
            while let Some(v) = rx.recv().await {
                yield v;
            }
        };

        //create a streaming response
        HttpResponse::Ok()
            .content_type("application/json")
            .streaming(payload)
    } else {
        trace!(
            "client attempted to request file {} from {:?}, but that server isn't connected",
            file_id,
            server_id
        );
        HttpResponse::NotFound()
            .content_type("text/html")
            .body("requested resource not found, the server may not be connected")
    }
}

/// Download a file from a client
async fn __download(
    _: HttpRequest,
    path: Path<(ServerId, FileId)>,
    state: Data<State>,
) -> HttpResponse {
    let (server_id, file_id) = path.into_inner();
    trace!("download request recieved for {}, {}", server_id, file_id);

    //Check server is online
    let reader = state.servers.read().await;
    let server_online = reader.contains_key(&server_id); //Duplicate req #cd
    trace!("checking if server `{:?}` is online", &server_id);
    if server_online {
        trace!("server `{:?}` is online", &server_id);
        let (tx, mut rx) = mpsc::channel(100);

        let download_id = rand::thread_rng().gen();

        trace!("download id is {:?}", download_id);

        //Create a valid upload job
        state.requests.write().await.insert(download_id, tx);

        //Acquire channel to WS, and send upload req. to server
        trace!("uploading req to server");
        let msg = format!("{}/upload/{}", state.base_url, download_id);
        let connected_servers = state.servers.read().await;
        let uploader_ws = connected_servers.get(&server_id).unwrap(); //Duplicate req #cd
        uploader_ws
            .send(Message::UploadTo(file_id, msg))
            .await
            .unwrap();

        trace!("generating response payload and returning");

        let payload = async_stream::stream! {
            //XXX: enable timeouts
            while let Some(v) = rx.recv().await {
                yield v;
            }
        };

        //create a streaming response
        HttpResponse::Ok()
            .content_type("text/html")
            .streaming(payload)
    } else {
        trace!(
            "client attempted to request file {} from {:?}, but that server isn't connected",
            file_id,
            server_id
        );
        HttpResponse::NotFound()
            .content_type("text/html")
            .body("requested resource not found, the server may not be connected")
    }
}

/// Download a file from a client
#[get("/download/{server_id}/{file_id}")]
pub async fn download(
    req: HttpRequest,
    state: Data<State>,
    path: Path<(ServerId, FileId)>,
) -> impl actix_web::Responder {
    __download(req, path, state).await
}

#[get("/metadata/{server_id}/{file_id}")]
pub async fn metadata(
    req: HttpRequest,
    state: Data<State>,
    path: Path<(ServerId, FileId)>,
) -> impl actix_web::Responder {
    __metadata(req, path, state).await
}

#[cfg(test)]
mod test {
    // use super::__download;
}
