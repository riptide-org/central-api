/*
    Author: Josiah Bull

    This application is the central api of the file sharing system. 
    It aims to fufill the following basic spec: 
    - Accept websocket connections from server agent
        - Should issue a persistent ID for that server agent.
        - Ext, fingerprint the server agent in some way and store that data?
    - Serve the front end static information pages
    - Accept GET resquests to download a file
        - Should wait for paired POST
        - Check if the server is online
        - Support timeouts
    - Accept POST requests to upload a file
        - Supports paired get request
        - Gracefully handle failed/interrupted upload

    - Ext: opt-out webrtc for proper peer-to-peer?
*/

mod db;
mod handler;
mod error;
mod structs;

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot, RwLock};

use std::convert::Infallible;
use warp::{path, Filter, Rejection};

use warp::ws::{Message, WebSocket};
use futures::stream::BoxStream;
use bytes::{Bytes};

use std::net::SocketAddr;

type ServerAgents = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
type PendingStreams = Arc<RwLock<HashMap<usize, oneshot::Sender<BoxStream<'static, Result<Bytes, warp::Error>>>>>>;

static NEXT_STREAM_ID: AtomicUsize = AtomicUsize::new(0);

const SERVER_IP: &str = "127.0.0.1:3030";
const REQUEST_TIMEOUT_THRESHOLD: u64 = 5000; //millis

fn with_db(db_pool: db::DBPool) -> impl Filter<Extract = (db::DBPool,), Error = Infallible> + Clone {
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

    let ws = warp::path("server-register")
        .and(path::param::<usize>().map(Some).or_else(|_| async { Ok::<(Option<usize>,), std::convert::Infallible>((None,)) })) //Optional usize parameter
        .and(path::end())
        .and(warp::ws())
        .and(with_db(db_pool.clone()))
        .and(agents.clone())
        .map(|id: Option<usize>, ws: warp::ws::Ws, db, a| {
            ws.on_upgrade(move |s| handler::websocket(s, id, db, a))
        });

    let upload = warp::post()
        .and(warp::path("upload"))
        .and(path::param())
        .and(path::end())
        .and(warp::filters::body::stream())
        .and(streams.clone())
        .and_then(handler::upload);
    
    let download = warp::get()
        .and(warp::path("download"))
        .and(path::param())
        .and(path::param())
        .and(path::end())
        .and(agents)
        .and(streams.clone())
        .and_then(handler::download);

    let heartbeat = warp::any()
        .and(warp::path("heartbeat"))
        .and(path::end())
        .and_then(handler::heartbeat);

    //Non-Api requests, this will be like the front end website etc
    let root = warp::any()
        .map(|| warp::reply::reply());

    let routes = heartbeat.or(ws).or(upload).or(download).with(warp::cors().allow_any_origin()).recover(error::handle_rejection);

    warp::serve(routes)
        .run(SERVER_IP.parse::<SocketAddr>().expect("Failed to parse address"))
        .await;
}