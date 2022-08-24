use actix_web::{
    get,
    web::{Data, Path, Bytes},
    HttpRequest, HttpResponse,
};
use log::trace;
use rand::Rng;
use tokio::{sync::mpsc, time};
use ws_com_framework::{FileId, Message};

use crate::{ServerId, State, websockets::InternalComm};

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
            .send(InternalComm::SendMessage(Message::MetadataReq(file_id, download_id)))
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
            .send(InternalComm::SendMessage(Message::UploadTo(file_id, msg)))
            .await
            .unwrap();

        trace!("generating response payload and returning");

        let payload = async_stream::stream! {
            //XXX: enable timeouts
            tokio::select! {
                v = rx.recv() => {
                    if let Some(x) = v {
                        yield x;
                    }
                },
                _ = time::sleep(tokio::time::Duration::from_secs(5)) => {
                    yield Ok(Bytes::from_static(b"download failed"))
                }
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
#[get("/download/{server_id}/{file_id}")]
pub async fn download(
    _: HttpRequest,
    state: Data<State>,
    path: Path<(ServerId, FileId)>,
) -> impl actix_web::Responder {
    let (s_id, f_id) = path.into_inner();
    __download((s_id, f_id), state).await
}

#[get("/metadata/{server_id}/{file_id}")]
pub async fn metadata(
    _: HttpRequest,
    state: Data<State>,
    path: Path<(ServerId, FileId)>,
) -> impl actix_web::Responder {
    let (s_id, f_id) = path.into_inner();
    __metadata((s_id, f_id), state).await
}

#[cfg(test)]
#[cfg(not(tarpaulin_include))]
mod test {
    use actix_web::{
        body::MessageBody,
        error::PayloadError,
        http::StatusCode,
        web::{self, Bytes},
    };
    use futures::future;
    use std::collections::HashMap;
    use tokio::{
        pin,
        sync::{mpsc::Sender, RwLock},
    };
    use ws_com_framework::Message;

    use crate::{RequestId, State, websockets::InternalComm};

    use super::{__download, __metadata};

    /// Test downloading a file
    #[tokio::test]
    async fn test_download() {
        let server_id = 45;
        let file_id = 34;

        let state = web::Data::new(State {
            unauthenticated_servers: Default::default(),
            servers: Default::default(),
            requests: RwLock::new(HashMap::new()),
            base_url: "https://localhost:8080".into(),
        });

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        state.servers.write().await.insert(server_id, tx);

        let task_state = state.clone();
        let handle = tokio::task::spawn(async move {
            if let Some(InternalComm::SendMessage(Message::UploadTo(recv_file_id, recv_upload_url))) = rx.recv().await {
                //Validate that the url contains the upload_id somewhere
                let requests: HashMap<RequestId, Sender<Result<Bytes, PayloadError>>> =
                    task_state.requests.read().await.clone();
                let upload_id = requests.clone().into_keys().next().unwrap();
                let sender = requests.into_values().next().unwrap();

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
            unauthenticated_servers: Default::default(),
            servers: Default::default(),
            requests: RwLock::new(HashMap::new()),
            base_url: "https://localhost:8080".into(),
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
            unauthenticated_servers: Default::default(),
            servers: Default::default(),
            requests: RwLock::new(HashMap::new()),
            base_url: "https://localhost:8080".into(),
        });

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        state.servers.write().await.insert(server_id, tx);

        let task_state = state.clone();
        let handle = tokio::task::spawn(async move {
            if let Some(InternalComm::SendMessage(Message::MetadataReq(recv_file_id, recv_upload_id))) = rx.recv().await {
                //Validate that the url contains the upload_id somewhere
                let requests: HashMap<RequestId, Sender<Result<Bytes, PayloadError>>> =
                    task_state.requests.read().await.clone();
                let upload_id = requests.clone().into_keys().next().unwrap();
                let sender = requests.into_values().next().unwrap();

                assert_eq!(recv_file_id, file_id);
                assert_eq!(recv_upload_id, upload_id);

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
            unauthenticated_servers: Default::default(),
            servers: Default::default(),
            requests: RwLock::new(HashMap::new()),
            base_url: "https://localhost:8080".into(),
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
