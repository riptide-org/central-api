//! This is the central always-on api for file-share. It is designed to take requests from clients
//! and the front end api, returning useful information to users.
//! It is currently in a functional prototype stage of development, a lot of work is needed to 
//! finalize the api functionality.

mod db;
mod error;
mod handler;
mod structs;

use std::collections::HashMap;
use std::sync::{atomic::AtomicUsize, Arc};
use tokio::sync::{mpsc, oneshot, RwLock};
use std::convert::Infallible;
use warp::{path, Filter};
use warp::ws::Message;
use std::net::SocketAddr;

/// All currently connected server agents. Each server agent has a unique id, so there is no chance of collisions.
type ServerAgents = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;

/// Requests from clients which are waiting for a response. The response can come from either an http request,
/// or from a websocket.
type PendingStreams =
    Arc<RwLock<HashMap<usize, oneshot::Sender<Result<Box<dyn warp::Reply>, warp::http::Error>>>>>;

/// Tracks stream ids, assigning a unique id to each request.
static NEXT_STREAM_ID: AtomicUsize = AtomicUsize::new(0);

/// The IP of this server. Eventually should be loaded from disk.
const SERVER_IP: &str = "127.0.0.1:3030";

/// The time before a pending request fails.
const REQUEST_TIMEOUT_THRESHOLD: u64 = 5000; //millis

fn with_db(
    db_pool: db::DBPool,
) -> impl Filter<Extract = (db::DBPool,), Error = Infallible> + Clone {
    warp::any().map(move || db_pool.clone())
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let db_pool = db::create_pool().expect("failed to create db pool");
    db::init_db(&db_pool).await.expect("failed to initalize db");

    let agents = ServerAgents::default();
    let streams = PendingStreams::default();

    let agents = warp::any().map(move || agents.clone());
    let streams = warp::any().map(move || streams.clone());

    // Get metadata for a file. Will return an error in the event that file does not exist,
    // or if the request timed out.
    let meta = warp::get()
        .and(path("get-meta"))
        .and(path::param::<usize>())
        .and(path::param::<uuid::Uuid>())
        .and(path::end())
        .and(agents.clone())
        .and(streams.clone())
        .and_then(handler::get_meta);

    //Register a new server agent, get an id to connect across the websocket connection.
    let ws_register = warp::post()
        .and(path("ws-register"))
        .and(path::end())
        .and(warp::body::content_length_limit(1024 * 8)) //Limit content bodies to 8kb
        .and(warp::body::json::<structs::AgentRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(handler::register_websocket);

    //Handles incoming websocket connections.
    let ws = path("ws")
        .and(path::param::<usize>())
        .and(path::end())
        .and(warp::ws())
        .and(with_db(db_pool.clone()))
        .and(agents.clone())
        .and(streams.clone())
        .map(|id: usize, ws: warp::ws::Ws, db, a, b| {
            //TODO validate the id here, rather than in the handler itself
            ws.on_upgrade(move |s| handler::websocket(s, id, db, a, b))
        });

    //Takes an upload stream from a server agent, and tries to pair it with a download stream from a client.
    let upload = warp::post()
        .and(path("upload"))
        .and(path::param())
        .and(path::end())
        .and(warp::filters::body::stream())
        .and(streams.clone())
        .and_then(handler::upload);

    //This is the endpoint a client should hit when they want to download a file
    //Has potential to timeout or fail if a server agent is not online, does not exist
    //or does not send a valid data stream.
    let download = warp::get()
        .and(path("download"))
        .and(path::param::<usize>())
        .and(path::param::<uuid::Uuid>())
        .and(path::end())
        .and(agents)
        .and(streams.clone())
        .and_then(handler::download);

    //Check if the server is up, and any relevant data about it
    //Will allow checks on specific server agents, and checks on the database health
    let heartbeat = warp::any()
        .and(path("heartbeat"))
        .and(path::end())
        .and_then(handler::heartbeat);

    //404 handler
    let catcher = warp::any().map(|| {
        warp::reply::with_status("Not Found", warp::hyper::StatusCode::from_u16(404).unwrap())
    });

    //TODO implement this propery (i.e. have things behind a 'v1' flag if we ever need to update the api)
    let routes = heartbeat
        .or(ws_register)
        .or(ws)
        .or(meta)
        .or(upload)
        .or(download)
        .or(catcher)
        .with(warp::cors().allow_any_origin())
        .recover(error::handle_rejection);

    warp::serve(routes)
        .run(
            SERVER_IP
                .parse::<SocketAddr>()
                .expect("Failed to parse address"),
        )
        .await;
}
