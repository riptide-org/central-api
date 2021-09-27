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
use std::net::SocketAddr;
use ws_com_framework::Message;
use std::env;

/// All currently connected server agents. Each server agent has a unique id, so there is no chance of collisions.
type ServerAgents = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;

/// Requests from clients which are waiting for a response. The response can come from either an http request,
/// or from a websocket.
type PendingStreams =
    Arc<RwLock<HashMap<usize, oneshot::Sender<Result<Box<dyn warp::Reply>, warp::http::Error>>>>>;

/// Tracks stream ids, assigning a unique id to each request.
static NEXT_STREAM_ID: AtomicUsize = AtomicUsize::new(0);

macro_rules! load_env {
    ( $x:expr ) => {
        match env::var($x) {
            Ok(f) => {
                match f.parse() {
                    Ok(g) => g,
                    Err(_) =>  panic!("Enviroment variable `{}` not found!", $x),
                }
            },
            Err(_) => panic!("Enviroment variable `{}` not found!", $x)
        }
    };
}

#[derive(Clone)]
pub struct Config {
    server_ip: String,
    server_port: u16,

    database_host: String,
    database_port: u16,
    database_name: String,
    database_user: String,
    database_pass: String,

    browser_base_url: String,

    request_timeout_threshold: usize,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            server_ip: String::from("127.0.0.1"), 
            server_port: 3030,

            database_host: String::from("127.0.0.1"),
            database_port: 7877,
            database_name: String::from("postgres"),
            database_user: String::from("postgres"),
            database_pass: String::from(""),

            browser_base_url: String::from("http://localhost:3030"),

            request_timeout_threshold: 5000,
        }
    }
}

impl Config {
    pub fn from_env() -> Config {
        Config {
            server_ip: load_env!("HOST"),
            server_port: load_env!("PORT"),

            database_host: load_env!("DB_HOST"),
            database_port: load_env!("DB_PORT"),
            database_name: load_env!("DB_NAME"),
            database_user: load_env!("DB_USER"),
            database_pass: load_env!("DB_PASS"),

            browser_base_url: load_env!("BROWSER_BASE_URL"),

            request_timeout_threshold: load_env!("REQUEST_TIMEOUT_THRESHOLD"),
        }
    }
}

fn with_db(
    db_pool: db::DBPool,
) -> impl Filter<Extract = (db::DBPool,), Error = Infallible> + Clone {
    warp::any().map(move || db_pool.clone())
}

fn with_config(
    cfg: Config,
) -> impl Filter<Extract = (Config,), Error = Infallible> + Clone {
    warp::any().map(move || cfg.clone())
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let cfg = match cfg!(debug_assertions) {
        true => Config::default(),
        false => Config::from_env(),
    };

    let db_pool = db::create_pool(&cfg).expect("failed to create db pool");
    db::init_db(&db_pool).await.expect("failed to initalize db");

    let agents = ServerAgents::default();
    let streams = PendingStreams::default();

    let agents = warp::any().map(move || agents.clone());
    let streams = warp::any().map(move || streams.clone());


    // Get metadata for a file. Will return an error in the event that file does not exist,
    // or if the request timed out.
    let meta = warp::get()
        .and(path::param::<usize>())
        .and(path("file"))
        .and(path::param::<uuid::Uuid>())
        .and(path::end())
        .and(agents.clone())
        .and(streams.clone())
        .and(with_config(cfg.clone()))
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
        .and(with_db(db_pool))
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
        .and(path::param::<usize>())
        .and(path::param::<uuid::Uuid>())
        .and(path("download"))
        .and(path::end())
        .and(agents)
        .and(streams.clone())
        .and(with_config(cfg.clone()))
        .and_then(handler::download);

    //Check if the server is up, and any relevant data about it
    //Will allow checks on specific server agents, and checks on the database health
    let heartbeat = warp::any()
        .and(path("heartbeat"))
        .and(warp::path::param::<usize>().map(Some).or_else(|_| async { Ok::<(Option<usize>,), std::convert::Infallible>((None,)) }))
        .and(path::end())
        .and_then(handler::heartbeat);

    let ping = warp::any()
        .and(path("ping"))
        .and_then(handler::ping);

    //404 handler
    let catcher = warp::any().map(|| {
        warp::reply::with_status("Not Found", warp::hyper::StatusCode::from_u16(404).unwrap())
    });

    let server = warp::path("server").and(
        download
        .or(meta)
        .or(heartbeat)
    );
    let client = warp::path("client").and(
        upload
        .or(ws)
        .or(ws_register)
    );

    let routes = warp::path("api")
        .and(warp::path("v1"))
        .and(
            server
            .or(client)
            .or(catcher)
        )
        .with(warp::cors())
        .recover(error::handle_rejection);

    warp::serve(routes)
        .run(
            format!("{}:{}", &cfg.server_ip, &cfg.server_port)
                .parse::<SocketAddr>()
                .expect("Failed to parse address"),
        )
        .await;
}
