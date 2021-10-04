//! Contains all handlers which are mapped to the request endpoints on the public api.

use crate::db::*;
use crate::structs::*;
use crate::{error::Error, Config, PendingStreams, ServerAgents};
use futures::{StreamExt, TryFutureExt};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::hyper::{Body, Response as Builder, StatusCode};
use warp::reject;
use warp::ws::WebSocket;
use warp::{reply::Response, Rejection};
use ws_com_framework::message::{Message, Request};
use ws_com_framework::{Receiver, Sender};

/// Checks the health of various services in relation to the central api
pub async fn heartbeat(_: Option<usize>) -> Result<Response, Rejection> {
    //Should do a check of the database here too
    //Maybe also enable an optional param to check on a specific server agent?
    Ok(warp::reply::Response::default())
}

/// The upload endpoint, designed to take a stream from a server agent and serve it
/// to a waiting client.
#[allow(unused_must_use)]
pub async fn upload(
    stream_id: String,
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
async fn cleanup(upload_id: String, streams: PendingStreams) {
    streams.write().await.remove(&upload_id);
}

/// A handler to return metadata about a file to a user.
pub async fn get_meta(
    agent_id: String,
    file_id: String,
    agents: ServerAgents,
    streams: PendingStreams,
    cfg: Config,
) -> Result<impl warp::Reply, Rejection> {
    fn send(
        agent: &mpsc::UnboundedSender<Message>,
        file_id: &str,
        upload_id: &str,
        _: &Config,
    ) -> Result<(), mpsc::error::SendError<Message>> {
        agent.send(Message::MetadataRequest(Request::new(
            file_id.to_owned(),
            upload_id.to_string(),
        )))
    }
    __download(agent_id, file_id, agents, streams, send, cfg).await
}

/// A handler for clients to request a file download. Will send a message through the websocket to a server agent
/// Who will then upload the fiel to the user.
pub async fn download(
    agent_id: String,
    file_id: String,
    agents: ServerAgents,
    streams: PendingStreams,
    cfg: Config,
) -> Result<impl warp::Reply, Rejection> {
    fn send(
        agent: &mpsc::UnboundedSender<Message>,
        file_id: &str,
        upload_id: &str,
        cfg: &Config,
    ) -> Result<(), mpsc::error::SendError<Message>> {
        let http_variant = match cfg.secure {
            true => "https",
            false => "http",
        };
        let url = format!(
            "{}/api/v1/client/upload/{}",
            cfg.browser_base_url, &upload_id
        ); //Could do with a toggle here for http vs https
        agent.send(Message::UploadRequest(Request::new(
            file_id.to_owned(),
            url,
        )))
    }
    __download(agent_id, file_id, agents, streams, send, cfg).await
}

/// An internal helper function, which can generically be called to either get metadata, or to trigger a file upload
/// depending on where it is called from.
async fn __download<T>(
    agent_id: String,
    file_id: String,
    agents: ServerAgents,
    streams: PendingStreams,
    send: T,
    cfg: Config,
) -> Result<impl warp::Reply, Rejection>
where
    T: Fn(
        &tokio::sync::mpsc::UnboundedSender<Message>,
        &str,
        &str,
        &Config,
    ) -> Result<(), mpsc::error::SendError<Message>>,
{
    let (tx, rx) = oneshot::channel();
    let upload_id = crate::common::get_random_hex(40);

    if let Some(agent) = agents.read().await.get(&agent_id) {
        streams.write().await.insert(upload_id.clone(), tx);
        if let Err(e) = send(agent, &file_id, &upload_id, &cfg) {
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

    return match timeout(
        Duration::from_millis(cfg.request_timeout_threshold as u64),
        rx,
    )
    .await
    {
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
pub async fn register_websocket(db: DBPool) -> Result<impl warp::Reply, Rejection> {
    let mut agent_req = AgentRequest::default();

    //Ensure that we have a unique public id
    while Search::PublicId(agent_req.public_id().to_string())
        .find(&db)
        .await?
        .is_some()
    {
        agent_req = AgentRequest::default();
    }

    let agent = match add_agent(&db, &agent_req).await {
        Ok(f) => f,
        Err(e) => return Err(warp::reject::custom(e)),
    };

    let response = format!(
        "{{\"public_id\": \"{}\", \"private_key\": \"{}\"}}",
        agent.public_id(),
        agent_req.secure_key_plain()
    );

    Ok(warp::reply::with_status(
        warp::reply::with_header(response, "content-type", "application/json"),
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
    public_id: String,
    db: DBPool,
    agents: ServerAgents,
    streams: PendingStreams,
) {
    //Largely copied from the proof of concept, some adjustments should be made to improve it at some point.
    //Probably remove use of unbounded channels to ensure no memory leaks for long-running connected clients
    //sending a lot of messages.

    //Check that this user exists and isn't online
    let agent = match Search::PublicId(public_id.clone()).find(&db).await {
        Ok(Some(a)) => {
            //Agent exists, but we need to check they aren't already logged in
            if agents.read().await.contains_key(&public_id) {
                return close_ws_conn(
                    ws,
                    format!("Error, agent {} attempted to connect twice!", &a).as_str(),
                )
                .await;
            }
            a
        }
        Ok(None) => return close_ws_conn(ws, "User attempted to connect with incorrect id.").await,
        Err(e) => return close_ws_conn(ws, format!("Error: {}", e).as_str()).await,
    };

    eprintln!("new server connected: {}", agent.public_id());

    //Split the streams up
    let (tx, rx) = ws.split();
    let mut tx_cent = Sender::new(tx);
    let mut rx_cent = Receiver::new(rx);

    //Carry out authentication
    //TODO put this in it's own function
    tx_cent
        .send(Message::AuthReq)
        .await
        .expect("Unable to send authreq");
    if let Some(auth) = rx_cent.next().await {
        let auth = match auth {
            Ok(f) => f,
            Err(e) => {
                return println!(
                    "Error trying to validate user agent {}, garbled input recieved: {:?}",
                    &public_id, e
                );
            }
        };

        if let Message::AuthResponse(res) = auth {
            //TODO eventually this will be a string, not u8!
            let temporary_u8_array: &[u8] = &res.key;
            let temporary_comparison_string: String =
                std::str::from_utf8(temporary_u8_array).unwrap().to_owned();
            let hashed_temporary_comparison_string =
                crate::common::hash(&temporary_comparison_string);
            if hashed_temporary_comparison_string != agent.secure_key_hashed() {
                return println!(
                    "Websocket {} sent incorrect secure key, unable to validate!",
                    &public_id
                );
            }
        } else {
            return println!("Websocket {} sent invalid data down pipeline. Recieved message {:?} instead of auth response.", &public_id, auth);
        }
    } else {
        return println!(
            "Websocket {} disconnected before performing validation handshake!",
            &public_id
        );
    }

    //Update db as this account has succesfully logged in
    if let Err(e) = update_agent(
        &db,
        &public_id,
        AgentUpdateRequest::new(chrono::offset::Utc::now()),
    )
    .await
    {
        return println!(
            "Error {} updating agent {} most recent login",
            e, &public_id
        );
    };

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
        .insert(agent.public_id().to_owned(), tx);

    //Handle messages recieved from the websocket, while it remains connected.
    while let Some(result) = rx_cent.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                eprintln!("websocket error(uid={}): {:?}", agent.public_id(), e);
                continue;
            }
        };

        match msg {
            Message::Error(e) => eprintln!("Error from agent id{}: '{:?}'", agent.public_id(), e),
            Message::Message(e) => {
                println!("Message from agent id{}: '{}'", agent.public_id(), e)
            }
            Message::MetadataResponse(f) => {
                if let Some(channel) = streams.write().await.remove(f.get_stream_id()) {
                    let json = warp::reply::json(&f.get_payload());
                    channel
                        .send(Ok(Box::new(warp::reply::with_status(
                            json,
                            StatusCode::from_u16(200).unwrap(),
                        ))))
                        .unwrap_or_else(|_| eprintln!("Failed to send data down channel"));
                }
            }
            Message::Close(c) => {
                println!("I was told to close! Message: {}", c);
                break;
            }
            _ => panic!(
                "Recieved unknown request: {:?} from agent id{}! This shouldn't happen!",
                msg,
                agent.public_id()
            ),
        }
    }

    //Server Disconnected
    eprintln!(
        "good bye server: {}, public id {}",
        agent.public_id(),
        agent.public_id()
    );
    agents.write().await.remove(agent.public_id());
}
