//! Contains all handlers which are mapped to the request endpoints on the public api.

use crate::db::*;
use crate::structs::*;
use crate::{error::Error, PendingStreams, ServerAgents, NEXT_STREAM_ID, Config};
use futures::{StreamExt, TryFutureExt};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::hyper::{Body, Response as Builder, StatusCode};
use warp::reject;
use warp::ws::{WebSocket};
use warp::{reply::Response, Rejection};
use ws_com_framework::{Sender, Receiver};
use ws_com_framework::message::{Message, FileRequest, FileUploadRequest};

/// Checks the health of various services in relation to the central api
pub async fn heartbeat() -> Result<Response, Rejection> {
    //Should do a check of the database here too
    //Maybe also enable an optional param to check on a specific server agent?
    Ok(warp::reply::Response::default())
}

/// The upload endpoint, designed to take a stream from a server agent and serve it
/// to a waiting client.
#[allow(unused_must_use)]
pub async fn upload(
    stream_id: usize,
    data: impl futures::Stream<Item = Result<impl bytes::buf::Buf, warp::Error>>
        + std::marker::Send
        + 'static,
    streams: PendingStreams,
) -> Result<impl warp::Reply, Rejection> {
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
        let body = std::boxed::Box::new(
            warp::hyper::Response::builder()
                .status(warp::hyper::StatusCode::OK)
                .body(warp::hyper::Body::wrap_stream(data)),
        );

        channel
            .send(Ok(body))
            .unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
    } else {
        return Err(reject::custom(Error::NoStream));
    }

    tokio_stream::wrappers::ReceiverStream::new(rx).next().await;
    Ok(Builder::builder()
        .status(StatusCode::OK)
        .body(Body::from("{\"message\": \"upload completed\"}")))
}

/// A helper function which removes a stream from the pending streams list.
async fn cleanup(upload_id: usize, streams: PendingStreams) {
    streams.write().await.remove(&upload_id);
}

/// A handler to return metadata about a file to a user.
pub async fn get_meta(
    agent_id: usize,
    file_id: uuid::Uuid,
    agents: ServerAgents,
    streams: PendingStreams,
    cfg: Config
) -> Result<impl warp::Reply, Rejection> {
    fn send(
        agent: &mpsc::UnboundedSender<Message>,
        file_id: uuid::Uuid,
        upload_id: usize,
        _: &Config
    ) -> Result<(), mpsc::error::SendError<Message>> {
        agent.send(
            FileRequest::new(file_id, upload_id).unwrap().into()
        )
    }
    __download(agent_id, file_id, agents, streams, send, cfg).await
}

/// A handler for clients to request a file download. Will send a message through the websocket to a server agent
/// Who will then upload the fiel to the user.
pub async fn download(
    agent_id: usize,
    file_id: uuid::Uuid,
    agents: ServerAgents,
    streams: PendingStreams,
    cfg: Config
) -> Result<impl warp::Reply, Rejection> {
    fn send(
        agent: &mpsc::UnboundedSender<Message>,
        file_id: uuid::Uuid,
        upload_id: usize,
        cfg: &Config,
    ) -> Result<(), mpsc::error::SendError<Message>> {
        let url = format!("http://{}:{}/upload/{}", cfg.server_ip, cfg.server_port, &upload_id); //Could do with a toggle here for http vs https
        agent.send(
            FileUploadRequest::new(file_id, url).into(),
        )
    }
    __download(agent_id, file_id, agents, streams, send, cfg).await
}

/// An internal helper function, which can generically be called to either get metadata, or to trigger a file upload
/// depending on where it is called from.
async fn __download<T>(
    agent_id: usize,
    file_id: uuid::Uuid,
    agents: ServerAgents,
    streams: PendingStreams,
    send: T,
    cfg: Config,
) -> Result<impl warp::Reply, Rejection>
where
    T: Fn(
        &tokio::sync::mpsc::UnboundedSender<Message>,
        uuid::Uuid,
        usize,
        &Config,
    ) -> Result<(), mpsc::error::SendError<Message>>,
{
    let (tx, rx) = oneshot::channel();
    let upload_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);

    if let Some(agent) = agents.read().await.get(&agent_id) {
        streams.write().await.insert(upload_id, tx);
        if let Err(e) = send(agent, file_id, upload_id, &cfg) {
            cleanup(upload_id, streams).await;
            return Err(reject::custom(Error::Server(e.to_string())));
        }
    } else {
        eprintln!(
            "User attempted to download a file from server which isn't online or doesn't exist!"
        );
        //TODO return a useful error depending if the server is online, or doesn't exist.
        return Err(warp::reject());
    };

    return match timeout(Duration::from_millis(cfg.request_timeout_threshold as u64), rx).await {
        Ok(f) => {
            Ok(f.unwrap()) //TODO error handling here
        }
        Err(_) => {
            cleanup(upload_id, streams).await;
            Err(reject::custom(Error::StreamTimeout))
        }
    };
}

/// Called to register a new websocket. A client must provide a valid id for the registration to be succesful.
/// This comes in the form of a string.
pub async fn register_websocket(
    r: AgentRequest,
    db: DBPool,
) -> Result<impl warp::Reply, Rejection> {
    let agent = match add_agent(&db, r).await {
        Ok(f) => f,
        Err(e) => return Err(warp::reject::custom(e)),
    };

    let json = warp::reply::json(&JsonResponse::new(agent.id().to_string()));
    Ok(warp::reply::with_status(
        json,
        StatusCode::from_u16(201).unwrap(),
    ))
}

/// A helper function to close a websocket connection, sending appropriate close signals in the process. 
async fn close_ws_conn(ws: WebSocket, msg: &str) {
    eprintln!("{}", msg);
    let (tx, rx) = ws.split();
    let mut tx = Sender::new(tx);
    tx.send(msg.to_owned())
        .await
        .expect("Failed to send closing message to websocket.");
    futures::stream::SplitSink::reunite(tx.underlying(), rx)
        .expect("Failed to reuinte streams for closing")
        .close()
        .await
        .expect("Failed to reuinte and close websocket streams.");
}

/// Handles incoming websocket connections, validating them and splitting sink/stream into different threads.
pub async fn websocket(
    ws: WebSocket,
    id: usize,
    db: DBPool,
    agents: ServerAgents,
    streams: PendingStreams,
) {
    //Largely copied from the proof of concept, some adjustments should be made to improve it at some point.
    //Probably remove use of unbounded channels to ensure no memory leaks for long-running connected clients
    //sending a lot of messages.

    //Validate this websocket connection
    let agent = match Search::Id(id).find(&db).await {
        Ok(Some(a)) => {
            if let Err(e) = update_agent(
                &db,
                &id,
                AgentUpdateRequest::new(chrono::offset::Utc::now()),
            )
            .await
            {
                return close_ws_conn(
                    ws,
                    format!("Error adding new agent: {} agent: {}", e, a).as_str(),
                )
                .await;
            };
            a
        }
        Ok(None) => return close_ws_conn(ws, "User attempted to connect with incorrect id.").await,
        Err(e) => return close_ws_conn(ws, format!("Error: {}", e).as_str()).await,
    };


    eprintln!("new server connected: {}", agent.id());

    //Split the streams up
    let (tx, rx) = ws.split();
    let mut tx_cent = Sender::new(tx);
    let mut rx_cent = Receiver::new(rx);

    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    //Handle sending information down the socket connection
    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            tx_cent
                .send(message)
                .unwrap_or_else(|e| {
                    eprintln!("websocket send error: {:?}", e);
                })
                .await;
        }
    });

    //List this server agent globally so that other requests may discover it.
    agents
        .write()
        .await
        .insert(agent.id().to_owned() as usize, tx);

    //Handle messages recieved from the websocket, while it remains connected.
    while let Some(result) = rx_cent.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                eprintln!("websocket error(uid={}): {:?}", agent.id(), e);
                continue;
            }
        };

        match msg {
            Message::Error(e) => eprintln!("Error from agent id{}: '{:?}'", agent.id(), e),
            Message::Message(e) => {
                println!("Message from agent id{}: '{}'", agent.id(), e)
            },
            Message::File(f) => {
                if let Some(channel) = streams.write().await.remove(&f.stream_id()) {
                    let json = warp::reply::json(&f);
                    channel
                        .send(Ok(Box::new(warp::reply::with_status(
                            json,
                            StatusCode::from_u16(200).unwrap(),
                        ))))
                        .unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
                }
            },
            Message::Close(c) => {
                println!("I was told to close! Message: {}", c);
                break;
            },
            _ => panic!(
                "Recieved unknown request: {:?} from agent id{}! This shouldn't happen!",
                msg,
                agent.id()
            ),
        }
    }

    //Server Disconnected
    eprintln!("good bye server: {}", agent.id());
    agents.write().await.remove(&(agent.id() as usize));
}
