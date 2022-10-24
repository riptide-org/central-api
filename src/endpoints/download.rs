//! Handles downloading of various items from the central api

use std::time::Duration;

use actix_web::{
    body::SizedStream,
    get,
    web::{self, Data, Path},
    HttpResponse,
};
use log::{error, trace, warn};
use rand::Rng;
use tokio::sync::mpsc;
use ws_com_framework::{error::ErrorKind, FileId, Message};

use crate::{config::Config, endpoints::websockets::InternalComm, ServerId, State};

//TODO: lot of duplicate code in the __download and __metadata functions, both internally and between them.
// needs refactoring

/// configure the download and metadata services
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(download).service(metadata);
}

async fn __metadata(path: (ServerId, FileId), state: Data<State>) -> HttpResponse {
    let (server_id, file_id) = path;
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
            .send(InternalComm::SendMessage(Message::MetadataReq {
                file_id,
                upload_id: download_id,
            }))
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
            .content_type("plain/text")
            .body("requested resource not found, the server may not be connected")
    }
}

/// Download a file from a client
async fn __download(
    path: (ServerId, FileId),
    state: Data<State>,
    config: Data<Config>,
) -> HttpResponse {
    let (server_id, file_id) = path;
    trace!("download request received for {}, {}", server_id, file_id);

    //Check server is online
    let reader = state.servers.read().await;
    let server_online = reader.contains_key(&server_id); //Duplicate req #cd
    trace!("checking if server `{:?}` is online", &server_id);
    if !server_online {
        trace!(
            "client attempted to request file {} from {:?}, but that server isn't connected",
            file_id,
            server_id
        );

        return HttpResponse::NotFound()
            .content_type("plain/text")
            .body("requested resource not found, the server may not be connected");
    }

    trace!("server `{:?}` is online", &server_id);

    // create metadata request
    let (meta_tx, mut meta_rx) = mpsc::channel(100);
    let meta_id = rand::thread_rng().gen();
    state.requests.write().await.insert(meta_id, meta_tx);

    //Acquire channel to WS, and send upload req. to server
    trace!("uploading req to server");
    let connected_servers = state.servers.read().await;
    let uploader_ws = connected_servers.get(&server_id).unwrap(); //Duplicate req #cd
    uploader_ws
        .send(InternalComm::SendMessage(Message::MetadataReq {
            file_id,
            upload_id: meta_id,
        }))
        .await
        .unwrap();

    let mut raw_metadata = match tokio::time::timeout(
        Duration::from_secs(config.request_timeout_seconds),
        meta_rx.recv(),
    )
    .await
    {
        Ok(Some(Ok(p))) => p.to_vec(),
        Ok(None) => {
            warn!("metadata request closed early...");
            // return error to client
            return HttpResponse::NotFound()
                .content_type("plain/text")
                .body("requested resource not found");
        }
        _ => {
            // remove request from server
            if state.requests.write().await.remove(&meta_id).is_none() {
                warn!("Tried to remove request that was no longer there due to timing out, but it wasn't there at all?!");
            };

            // send error to agent
            if let Err(e) = uploader_ws
                .send(InternalComm::SendMessage(Message::Error {
                    kind: ErrorKind::FailedFileUpload,
                    reason: Some(String::from("failed to upload in time")),
                }))
                .await
            {
                error!(
                    "failed to send failed upload error response to client {:?}, error: {}",
                    server_id, e
                );
            }

            // return error to client
            return HttpResponse::NotFound()
                .content_type("plain/text")
                .body("requested resource not found, the server may not be connected");
        }
    };

    // wait for metadata
    while let Some(Ok(v)) = meta_rx.recv().await {
        raw_metadata.extend_from_slice(&v);
    }

    // parse metadata //TODO: instead of using serde_json here - we should define a global struct
    // which we can use to parse/unparse the data
    let raw_metadata: serde_json::Value = serde_json::from_slice(&raw_metadata).unwrap();
    let file_name = raw_metadata["file_name"].as_str().unwrap();
    let file_size = raw_metadata["file_size"].as_u64().unwrap();

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
    if let Err(e) = uploader_ws
        .send(InternalComm::SendMessage(Message::UploadTo {
            file_id,
            upload_url: msg,
        }))
        .await
    {
        error!("failed to send upload request to server: {}", e);

        // return error to client
        return HttpResponse::NotFound()
            .content_type("plain/text")
            .body("requested resource not found, the server may not be connected");
    }

    trace!("generating response payload and returning");

    // wait for the first message on rx
    let initial_payload = match tokio::time::timeout(
        Duration::from_secs(config.request_timeout_seconds),
        rx.recv(),
    )
    .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            // return error to client
            return HttpResponse::NotFound()
                .content_type("plain/text")
                .body("requested resource not found");
        }
        _ => {
            // remove request from server
            if state.requests.write().await.remove(&download_id).is_none() {
                warn!("Tried to remove request that was no longer there due to timing out, but it wasn't there at all?!");
            };

            // send error to agent
            if let Err(e) = uploader_ws
                .send(InternalComm::SendMessage(Message::Error {
                    kind: ErrorKind::FailedFileUpload,
                    reason: Some(String::from("failed to upload in time")),
                }))
                .await
            {
                error!(
                    "failed to send failed upload error response to client {:?}, error: {}",
                    server_id, e
                );
            }

            // return error to client
            return HttpResponse::NotFound()
                .content_type("plain/text")
                .body("requested resource not found, the server may not be connected");
        }
    };

    let payload = async_stream::stream! {
        // XXX: apply the above to the 3 other locations (download/metadata/info)
        yield initial_payload;
        while let Some(x) = rx.recv().await {
            yield x;
        }
    };

    let sized_stream = SizedStream::new(file_size, payload);

    //create a streaming response
    HttpResponse::Ok()
        .content_type("application/octet-stream")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", file_name),
        ))
        .body(sized_stream)
}

/// Download a file from a client
#[get("/agents/{server_id}/files/{file_id}")]
async fn download(
    state: Data<State>,
    config: Data<Config>,
    path: Path<(ServerId, FileId)>,
) -> impl actix_web::Responder {
    let (s_id, f_id) = path.into_inner();
    __download((s_id, f_id), state, config).await
}

#[get("/agents/{server_id}/files/{file_id}/metadata")]
async fn metadata(state: Data<State>, path: Path<(ServerId, FileId)>) -> impl actix_web::Responder {
    let (s_id, f_id) = path.into_inner();
    __metadata((s_id, f_id), state).await
}
