use crate::{ServerAgents, PendingStreams, NEXT_STREAM_ID, SERVER_IP, REQUEST_TIMEOUT_THRESHOLD, error::Error};
use crate::db::*;
use crate::structs::AgentUpdateRequest;
use warp::{reply::Response, Rejection};
use warp::ws::{Message, WebSocket};
use warp::hyper::{Response as Builder, StatusCode, Body};
use std::sync::atomic::Ordering;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use futures::{SinkExt, StreamExt, TryFutureExt, Stream};
use std::time::Duration;
use warp::reject;

pub async fn heartbeat() -> Result<Response, Rejection> {
    //Should do a check of the database here too

    Ok(warp::reply::Response::default())
}

#[allow(unused_must_use)]
pub async fn upload(stream_id: usize, data: impl futures::Stream<Item = Result<impl bytes::buf::Buf, warp::Error>> + std::marker::Send + 'static, streams: PendingStreams) -> Result<impl warp::Reply, Rejection> {
    //This extra channel is used to detect the completion of the download stream, so we can then return upload 
    let (tx, rx) = mpsc::channel::<()>(1);
    let data = data
        .map(|d| d.map(|mut b| b.copy_to_bytes(b.remaining())))
        .take_while(move |r| {
            tx.send(()); //No await needed, as this is just a delay util.
            futures::future::ready(r.is_ok())
        })
        .boxed();

    if let Some(channel) = streams.write().await.remove(&stream_id) {
        channel.send(data).unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
    } else {
        return Err(reject::custom(Error::NoStream));
    }

    tokio_stream::wrappers::ReceiverStream::new(rx).next().await;
    Ok(Builder::builder()
        .status(StatusCode::OK)
        .body(Body::from("{\"message\": \"upload completed\"}"))
    )
}

async fn cleanup(upload_id: usize, streams: PendingStreams) {
    streams.write().await.remove(&upload_id);
}

pub async fn download(agent_id: usize, file_id: usize, agents: ServerAgents, streams: PendingStreams) -> Result<impl warp::Reply, Rejection> {
    let (tx, rx) = oneshot::channel();
    let upload_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);

    if let Some(agent) = agents.read().await.get(&agent_id) {
        let url = format!("http://{}/upload/{}", SERVER_IP, &upload_id); //Could do with a toggle here for http vs https

        streams.write().await.insert(upload_id, tx);

        if let Err(e) = agent.send(Message::text(format!("{{\"send_file\": \"{}\",  \"file_id\": \"{}\"}}", url, file_id))) {
            cleanup(upload_id, streams).await;
            return Err(reject::custom(Error::ServerError(e.to_string())));
        }
    } else {
        eprintln!("User attempted to download a file from server which isn't online or doesn't exist!"); 
        //TODO return a useful error depending if the server is online, or doesn't exist.
        return Err(warp::reject());
    };

    return match timeout(Duration::from_millis(REQUEST_TIMEOUT_THRESHOLD), rx).await {
        Ok(f) => {
            Ok(warp::hyper::Response::builder()
                .status(warp::hyper::StatusCode::OK)
                .body(warp::hyper::Body::wrap_stream(
                    f.map_err(|e| Error::StreamError(e))?
                )))
        },
        Err(_) => {
            cleanup(upload_id, streams).await;
            Err(reject::custom(Error::StreamTimeout))
        }
    }
}

pub async fn static_file() -> Result<Response, Rejection> {

    Err(warp::reject())
}

async fn close_ws_conn(ws: WebSocket, msg: &str) -> () {
    //TODO error handling
    eprintln!("{}" , msg);
    let (mut tx, rx) = ws.split();
    tx.send(Message::text(msg)).await;
    futures::stream::SplitSink::reunite(tx, rx).expect("Failed to reuinte streams for closing").close().await;
}

pub async fn websocket(ws: WebSocket, unparsed_id: Option<usize>, db: DBPool, agents: ServerAgents) {
    //Largely copied from the proof of concept, some adjustments should be made to improve it at some point.
    //Probably remove use of unbounded channels to ensure no memory leaks for long-running connected clients
    //sending a lot of messages.

    let agent: crate::structs::Agent;
    if let Some(id) = unparsed_id {
        match Search::Id(id as i64).find(&db).await {
            Ok(Some(a)) => {
                if let Err(e) = update_agent(&db, &(id as i64), AgentUpdateRequest::new(chrono::offset::Utc::now())).await {
                    return close_ws_conn(ws, format!("Error: {}", e).as_str()).await;
                };
                agent = a;
            },
            Ok(None) => return close_ws_conn(ws, "User attempted to connect with incorrect id.").await,
            Err(e) => return close_ws_conn(ws, format!("Error: {}", e).as_str()).await,
        }
    } else {
        //This is a new user registration
        let new_agent = crate::structs::AgentRequest::new(uuid::Uuid::new_v4().to_string());
        agent = match add_agent(&db, new_agent).await {
            Ok(f) => f,
            Err(e) => return close_ws_conn(ws, format!("Failed to register new user: {}", e).as_str()).await,
        }
    }

    let (mut user_ws_tx, mut user_ws_rx) = ws.split();

    eprintln!("new server connected: {}", agent.id());

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

    agents.write().await.insert(agent.id().to_owned() as usize, tx);

    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("websocket error(uid={}): {}", agent.id(), e);
                break;
            }
        };
        //This function will handle anything received by this server client over the websocket.
        //For the purposes of this demo, we're going to just burn the input.
        eprintln!("Message received from client: {}, content: {:?}", agent.id(), msg);
    }

    //Server Disconnected
    eprintln!("good bye server: {}", agent.id());
    agents.write().await.remove(&(agent.id() as usize));
}   