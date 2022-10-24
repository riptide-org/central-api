use std::time::Duration;

use actix_web::{web::Data, HttpResponse};
use async_stream::AsyncStream;
use log::{error, warn};
use tokio::sync::mpsc;
use ws_com_framework::{error::ErrorKind, Message};

use crate::{endpoints::websockets::InternalComm, ServerId, State};

pub(crate) async fn wait_for_bytes<T, E>(
    recv: &mut mpsc::Receiver<Result<T, E>>,
    timeout: u64,
) -> Result<T, HttpResponse> {
    match tokio::time::timeout(Duration::from_secs(timeout), recv.recv()).await {
        Ok(Some(Ok(b))) => Ok(b),
        Ok(None) => Err(HttpResponse::NotFound().finish()),
        _ => {
            error!("Error while waiting for bytes");
            Err(HttpResponse::RequestTimeout().finish())
        }
    }
}

pub(crate) async fn collect_bytes<T, E>(
    req_id: &ServerId,
    recv: &mut mpsc::Receiver<Result<T, E>>,
    timeout: u64,
    state: &Data<State>,
    uploader_ws: &mpsc::Sender<InternalComm>,
    server_id: &ServerId,
    limit: Option<usize>,
) -> Result<Vec<u8>, HttpResponse>
where
    T: Into<Vec<u8>>,
    E: std::error::Error,
{
    let first = match wait_for_bytes(recv, timeout).await {
        Ok(b) => b,
        Err(e) => {
            // remove request from server
            if state.requests.write().await.remove(req_id).is_none() {
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

            return Err(e);
        }
    };
    let mut bytes: Vec<u8> = first.into();
    while let Some(x) = recv.recv().await {
        match x {
            Ok(b) => {
                bytes.extend(b.into());
                if let Some(limit) = limit {
                    if bytes.len() > limit {
                        uploader_ws
                            .send(InternalComm::SendMessage(Message::Error {
                                kind: ErrorKind::FailedFileUpload,
                                reason: Some(String::from("file too large")),
                            }))
                            .await
                            .unwrap();

                        return Err(HttpResponse::PayloadTooLarge().finish());
                    }
                }
            }
            Err(e) => {
                error!("Error while collecting bytes: {}", e);
                return Err(HttpResponse::InternalServerError().finish());
            }
        }
    }
    Ok(bytes)
}

pub(crate) async fn construct_stream<'a, T: 'a, E: 'a>(
    req_id: &ServerId,
    mut recv: mpsc::Receiver<Result<T, E>>,
    timeout: u64,
    state: &Data<State>,
    uploader_ws: &mut mpsc::Sender<InternalComm>,
    server_id: &ServerId,
) -> Result<AsyncStream<Result<T, E>, impl futures::Future<Output = ()> + 'a>, HttpResponse> {
    let first = match wait_for_bytes(&mut recv, timeout).await {
        Ok(b) => b,
        Err(e) => {
            // remove request from server
            if state.requests.write().await.remove(req_id).is_none() {
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

            return Err(e);
        }
    };
    Ok(async_stream::stream! {
        yield Ok(first);
        while let Some(x) = recv.recv().await {
            yield x;
        }
    })
}
