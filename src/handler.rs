use crate::{ServerAgents, PendingStreams, NEXT_STREAM_ID, SERVER_IP, REQUEST_TIMEOUT_THRESHOLD};
use warp::{reply::Response, Rejection};
use warp::ws::{Message, WebSocket};
use warp::hyper::{Response as Builder, StatusCode, Body};
use std::sync::atomic::Ordering;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use futures::{SinkExt, StreamExt, TryFutureExt, Stream};
use std::time::Duration;

pub async fn heartbeat() -> Result<Response, Rejection> {
    //Should do a check of the database here too
    Ok(warp::reply::Response::default())
}

pub async fn upload(stream_id: usize, data: impl futures::Stream<Item = Result<impl bytes::buf::Buf, warp::Error>> + std::marker::Send + 'static, streams: PendingStreams) -> Result<Response, Rejection> {
    let (tx, rx) = mpsc::channel::<()>(1);
    let data = data
        .map(|d| d.map(|mut b| b.copy_to_bytes(b.remaining())))
        .take_while(move |r| {
            tx.send(());
            futures::future::ready(r.is_ok())
        })
        .boxed();

    if let Some(channel) = streams.write().await.remove(&stream_id) {
        channel.send(data).unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
    } else {
        eprintln!("Server agent attempted to upload file, but no stream was waiting!");
        return Err(warp::reject())
    }

    tokio_stream::wrappers::ReceiverStream::new(rx).next().await;
    Ok(Builder::builder()
        .status(StatusCode::OK)
        .body("File Uploaded".into())
        .unwrap()) //Error handling?
}

async fn cleanup(upload_id: usize, streams: PendingStreams) -> Result<(), ()> { //TODO error handling
    streams.write().await.remove(&upload_id); //Clear the upload id we created
    Ok(())
}

pub async fn download(agent_id: usize, file_id: usize, agents: ServerAgents, streams: PendingStreams) -> Result<impl warp::Reply, Rejection> {
    let (tx, rx) = oneshot::channel();

    if let Some(agent) = agents.read().await.get(&agent_id) {
        let upload_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);
        let url = format!("http://{}/upload/{}", SERVER_IP, &upload_id); //Could do with a toggle here for http vs https

        streams.write().await.insert(upload_id, tx);

        if let Err(e) = agent.send(Message::text(format!("{{\"send_file\": \"{}\",  \"file_id\": \"{}\"}}", url, file_id))) {
            //TODO Error handling - something has gone *terribly* wrong
            eprintln!("Error sending to websocket: {}", e);
            cleanup(upload_id, streams).await;
            return Err(warp::reject());
        }
    } else {
        eprintln!("User attempted to download a file from server which isn't online or doesn't exist!"); 
        return Err(warp::reject());
    };

    return match timeout(Duration::from_millis(REQUEST_TIMEOUT_THRESHOLD), rx).await {
        Ok(f) => {
            Ok(
                warp::hyper::Response::builder()
                .status(warp::hyper::StatusCode::OK)
                .body(warp::hyper::Body::wrap_stream(f.unwrap()))
                
            )
        },
        Err(e) => {
            eprintln!("Failed to find waiting stream!");
            Err(warp::reject()) //Error handling?
        }
    }
}

pub async fn static_file() -> Result<Response, Rejection> {

    Err(warp::reject())
}

pub async fn websocket(ws: WebSocket, agents: ServerAgents) {
    //Largely copied from the proof of concept, some adjustments should be made to improve it at some point.
    //Probably remove use of unbounded channels to ensure no memory leaks for long-running connected clients
    //sending a lot of messages.

    //Todo generate id, for now lets just reuse the stream_id generator. Probably use uuid eventually.
    let agent_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);

    eprintln!("new server connected: {}", agent_id);

    let (mut user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            user_ws_tx
                .send(message)
                .unwrap_or_else(|e| {
                    eprintln!("websocket send error: {}", e);
                })
                .await;
        }
    });

    agents.write().await.insert(agent_id, tx);

    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("websocket error(uid={}): {}", agent_id, e);
                break;
            }
        };
        //This function will handle anything received by this server client over the websocket.
        //For the purposes of this demo, we're going to just burn the input.
        eprintln!("Message received from client: {}, content: {:?}", agent_id, msg);
    }

    //Server Disconnected
    eprintln!("good bye server: {}", agent_id);
    agents.write().await.remove(&agent_id);
}   