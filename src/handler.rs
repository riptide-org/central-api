use crate::{ServerAgents, PendingStreams, NEXT_STREAM_ID, SERVER_IP, REQUEST_TIMEOUT_THRESHOLD, error::Error};
use crate::db::*;
use crate::structs::*;
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
use std::convert::TryFrom;
use ws_com_framework::WebsocketMessage;

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

        let body = std::boxed::Box::new(warp::hyper::Response::builder()
            .status(warp::hyper::StatusCode::OK)
            .body(warp::hyper::Body::wrap_stream(data)));

        channel
            .send(Ok(body))
            .unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
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

pub async fn get_meta(agent_id: usize, file_id: uuid::Uuid, agents: ServerAgents, streams: PendingStreams) -> Result<impl warp::Reply, Rejection> {
    fn send(agent: &mpsc::UnboundedSender<Message>, file_id: uuid::Uuid, upload_id: usize) -> Result<(), mpsc::error::SendError<Message>> {
        agent.send(WebsocketMessage::Request(ws_com_framework::FileRequest::new(file_id, upload_id).unwrap()).into())
    }
    __download(agent_id, file_id, agents, streams, send).await
}


pub async fn download(agent_id: usize, file_id: uuid::Uuid, agents: ServerAgents, streams: PendingStreams) -> Result<impl warp::Reply, Rejection> {
    fn send(agent: &mpsc::UnboundedSender<Message>, file_id: uuid::Uuid, upload_id: usize) -> Result<(), mpsc::error::SendError<Message>> {
        let url = format!("http://{}/upload/{}", SERVER_IP, &upload_id); //Could do with a toggle here for http vs https
        agent.send(WebsocketMessage::Upload(ws_com_framework::FileUploadRequest::new(file_id, url)).into())
    }
    __download(agent_id, file_id, agents, streams, send).await
}

async fn __download<T>(agent_id: usize, file_id: uuid::Uuid, agents: ServerAgents, streams: PendingStreams, send: T) -> Result<impl warp::Reply, Rejection>
where T: Fn(&mpsc::UnboundedSender<Message>, uuid::Uuid, usize) -> Result<(), mpsc::error::SendError<Message>>  {
    let (tx, rx) = oneshot::channel();
    let upload_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);

    if let Some(agent) = agents.read().await.get(&agent_id) {
        streams.write().await.insert(upload_id, tx);
        if let Err(e) = send(agent, file_id, upload_id) {
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
            Ok(f.unwrap()) //TODO error handling here
        },
        Err(_) => {
            cleanup(upload_id, streams).await;
            Err(reject::custom(Error::StreamTimeout))
        }
    }
}

pub async fn register_websocket(r: AgentRequest, db: DBPool) -> Result<impl warp::Reply, Rejection> {
    let agent = match add_agent(&db, r).await {
        Ok(f) => f,
        Err(e) => return Err(warp::reject::custom(e)),
    };

    let json = warp::reply::json(&JsonResponse::new(agent.id().to_string()));
    Ok(warp::reply::with_status(json, StatusCode::from_u16(201).unwrap()))
}

async fn close_ws_conn(ws: WebSocket, msg: &str) -> () {
    eprintln!("{}" , msg);
    let (mut tx, rx) = ws.split();
    tx.send(Message::text(msg)).await.expect("Failed to send closing message to websocket.");
    futures::stream::SplitSink::reunite(tx, rx).expect("Failed to reuinte streams for closing").close().await.expect("Failed to reuinte and close websocket streams.");
}

pub async fn websocket(ws: WebSocket, id: usize, db: DBPool, agents: ServerAgents, streams: PendingStreams) {
    //Largely copied from the proof of concept, some adjustments should be made to improve it at some point.
    //Probably remove use of unbounded channels to ensure no memory leaks for long-running connected clients
    //sending a lot of messages.
    let agent = match Search::Id(id).find(&db).await {
        Ok(Some(a)) => {
            if let Err(e) = update_agent(&db, &id, AgentUpdateRequest::new(chrono::offset::Utc::now())).await {
                return close_ws_conn(ws, format!("Error adding new agent: {} agent: {}", e, a).as_str()).await;
            };
            a
        },
        Ok(None) => return close_ws_conn(ws, "User attempted to connect with incorrect id.").await,
        Err(e) => return close_ws_conn(ws, format!("Error: {}", e).as_str()).await,
    };

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
            Ok(m) => m,
            Err(e) => {
                eprintln!("websocket error(uid={}): {}", agent.id(), e);
                break;
            }
        };

        let msg: WebsocketMessage = match WebsocketMessage::try_from(msg.clone()) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Received mangaled transmission from client: {}\nError: {}\nMessage: {:?}", agent.id(), e, msg);
                continue;
            }
        };

        match msg {
            WebsocketMessage::Error(e) => eprintln!("Error from agent id{}: '{}'", agent.id(), e),
            WebsocketMessage::Message(e) => println!("Message from agent id{}: '{}'", agent.id(), e),
            WebsocketMessage::File(f) => {
            if let Some(channel) = streams.write().await.remove(&f.stream_id()) {
                let json = warp::reply::json(&f);
                channel.send(Ok(Box::new(warp::reply::with_status(json, StatusCode::from_u16(200).unwrap())))).unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
            }
            },
            _ => panic!("Recieved unknown request: {} from agent id{}! This shouldn't happen!", msg, agent.id()),
        }
    }
    
    //Server Disconnected
    eprintln!("good bye server: {}", agent.id());
    agents.write().await.remove(&(agent.id() as usize));
}   