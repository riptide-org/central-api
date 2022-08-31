//! Handles downloading of various items from the central api

use std::time::Duration;

use actix_web::{
    get,
    web::{self, Data, Path},
    HttpResponse,
};
use log::{error, trace, warn};
use rand::Rng;
use tokio::sync::mpsc;
use ws_com_framework::{error::ErrorKind, FileId, Message};

use crate::{endpoints::websockets::InternalComm, ServerId, State};

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
            .content_type("application/octet-stream")
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
async fn __download(path: (ServerId, FileId), state: Data<State>) -> HttpResponse {
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
        Duration::from_secs(5),
        /* TODO: configurable timeout here */ rx.recv(),
    )
    .await
    {
        Ok(Some(p)) => p,
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

    //XXX: investigate insertion of a Content-Disposition header: https://stackoverflow.com/questions/386845/http-headers-for-file-downloads

    //create a streaming response
    HttpResponse::Ok()
        .content_type("application/octet-stream")
        .streaming(payload)
}

/// Download a file from a client
#[get("/agents/{server_id}/files/{file_id}")]
async fn download(state: Data<State>, path: Path<(ServerId, FileId)>) -> impl actix_web::Responder {
    let (s_id, f_id) = path.into_inner();
    __download((s_id, f_id), state).await
}

#[get("/agents/{server_id}/files/{file_id}/metadata")]
async fn metadata(state: Data<State>, path: Path<(ServerId, FileId)>) -> impl actix_web::Responder {
    let (s_id, f_id) = path.into_inner();
    __metadata((s_id, f_id), state).await
}

#[cfg(test)]
#[cfg(not(tarpaulin_include))]
mod test {
    use actix_web::{body::MessageBody, http::StatusCode, web};
    use futures::future;
    use tokio::{pin, sync::RwLock};
    use ws_com_framework::Message;

    use crate::{endpoints::websockets::InternalComm, timelocked_hashmap::TimedHashMap, State};

    use super::{__download, __metadata};

    /// Test downloading a file
    #[tokio::test]
    async fn test_download() {
        let server_id = 45;
        let file_id = 34;

        let state = web::Data::new(State {
            unauthenticated_servers: RwLock::new(TimedHashMap::new()),
            servers: Default::default(),
            requests: RwLock::new(TimedHashMap::new()),
            base_url: "https://localhost:8080".into(),
            start_time: std::time::Instant::now(),
        });

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        state.servers.write().await.insert(server_id, tx);

        let task_state = state.clone();
        let handle = tokio::task::spawn(async move {
            if let Some(InternalComm::SendMessage(Message::UploadTo {
                file_id: recv_file_id,
                upload_url: recv_upload_url,
            })) = rx.recv().await
            {
                //Validate that the url contains the upload_id somewhere
                let requests = task_state.requests.read().await;
                let upload_id = requests.keys().next().unwrap();
                let sender = requests.values().next().unwrap();

                assert_eq!(recv_file_id, file_id);
                assert!(recv_upload_url.contains(&format!("{upload_id}")));

                sender
                    .send(Ok(actix_web::web::Bytes::copy_from_slice(
                        b"some test data",
                    )))
                    .await
                    .unwrap();
            } else {
                panic!("sent no message, or wrong message");
            }
        });

        let resp = __download((server_id, file_id), state.clone()).await;

        handle.await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "application/octet-stream"
        );

        let body = resp.into_body();
        pin!(body);

        let bytes = future::poll_fn(|cx| body.as_mut().poll_next(cx)).await;
        assert_eq!(
            bytes.unwrap().unwrap(),
            web::Bytes::from_static(b"some test data")
        );
    }

    /// Test downloading a file without a server connected
    #[tokio::test]
    async fn test_download_disconnected() {
        let server_id = 45;
        let file_id = 34;

        let state = web::Data::new(State {
            unauthenticated_servers: RwLock::new(TimedHashMap::new()),
            servers: Default::default(),
            requests: RwLock::new(TimedHashMap::new()),
            base_url: "https://localhost:8080".into(),
            start_time: std::time::Instant::now(),
        });

        let resp = __download((server_id, file_id), state.clone()).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        assert_eq!(resp.headers().get("Content-Type").unwrap(), "plain/text");

        let body = resp.into_body();
        pin!(body);

        let bytes = future::poll_fn(|cx| body.as_mut().poll_next(cx)).await;
        assert_eq!(
            bytes.unwrap().unwrap(),
            web::Bytes::from_static(
                b"requested resource not found, the server may not be connected"
            )
        );
    }

    /// Test getting metadata
    #[tokio::test]
    async fn test_metadata() {
        let server_id = 45;
        let file_id = 34;

        let state = web::Data::new(State {
            unauthenticated_servers: RwLock::new(TimedHashMap::new()),
            servers: Default::default(),
            requests: RwLock::new(TimedHashMap::new()),
            base_url: "https://localhost:8080".into(),
            start_time: std::time::Instant::now(),
        });

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        state.servers.write().await.insert(server_id, tx);

        let task_state = state.clone();
        let handle = tokio::task::spawn(async move {
            if let Some(InternalComm::SendMessage(Message::MetadataReq {
                file_id: recv_file_id,
                upload_id: recv_upload_id,
            })) = rx.recv().await
            {
                //Validate that the url contains the upload_id somewhere
                let requests = task_state.requests.read().await;
                let upload_id = requests.keys().next().unwrap();
                let sender = requests.values().next().unwrap().clone();

                assert_eq!(recv_file_id, file_id);
                assert_eq!(recv_upload_id, *upload_id);

                sender
                    .send(Ok(actix_web::web::Bytes::copy_from_slice(
                        b"some test data",
                    )))
                    .await
                    .unwrap();
            } else {
                panic!("sent no message, or wrong message");
            }
        });

        let resp = __metadata((server_id, file_id), state.clone()).await;

        handle.await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body();
        pin!(body);

        let bytes = future::poll_fn(|cx| body.as_mut().poll_next(cx)).await;
        assert_eq!(
            bytes.unwrap().unwrap(),
            web::Bytes::from_static(b"some test data")
        );
    }

    /// Test getting metadata without a server connected
    #[tokio::test]
    async fn test_metadata_disconnected() {
        let server_id = 45;
        let file_id = 34;

        let state = web::Data::new(State {
            unauthenticated_servers: RwLock::new(TimedHashMap::new()),
            servers: Default::default(),
            requests: RwLock::new(TimedHashMap::new()),
            base_url: "https://localhost:8080".into(),
            start_time: std::time::Instant::now(),
        });

        let resp = __metadata((server_id, file_id), state.clone()).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        assert_eq!(resp.headers().get("Content-Type").unwrap(), "plain/text");

        let body = resp.into_body();
        pin!(body);

        let bytes = future::poll_fn(|cx| body.as_mut().poll_next(cx)).await;
        assert_eq!(
            bytes.unwrap().unwrap(),
            web::Bytes::from_static(
                b"requested resource not found, the server may not be connected"
            )
        );
    }
}
