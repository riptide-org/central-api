//! The central api which handles websockets and file streams between uploaders and clients

#![warn(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    // clippy::missing_docs_in_private_items,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications,
    deprecated
)]

#[macro_use]
extern crate diesel;

mod endpoints;
mod db;
mod error;
mod models;
#[cfg(not(tarpaulin_include))]
mod schema;

use actix_web::{
    error::PayloadError,
    middleware::Logger,
    web::{self, Bytes},
    App, HttpServer,
};
use db::{Database, DbBackend};
use dotenv::dotenv;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use endpoints::websockets::InternalComm as WsInternalComm;
use ws_com_framework::PublicId as ServerId;

type RequestId = u64;

/// State holds information about all current active connections and nodes
#[derive(Debug)]
pub struct State {
    /// Websockets that are connected, but have not yet completed authentication
    unauthenticated_servers: RwLock<HashMap<ServerId, mpsc::Sender<WsInternalComm>>>,
    /// Connected websockets that are valid, and have responded to a ping within the last X seconds
    servers: RwLock<HashMap<ServerId, mpsc::Sender<WsInternalComm>>>,
    /// Actively waiting requests that need an agent to respond - that also haven't timed out yet
    requests: RwLock<HashMap<RequestId, mpsc::Sender<Result<Bytes, PayloadError>>>>,
    /// The base URL of this server
    base_url: String,
    /// The instant that the server started
    start_time: std::time::Instant,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();

    // load .env file, and all parameters
    dotenv().ok();
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let domain = std::env::var("DOMAIN").expect("DOMAIN must be set");
    let port = std::env::var("PORT")
        .expect("PORT must be set")
        .parse()
        .expect("PORT must be a number");
    let host = std::env::var("HOST").expect("HOST must be set");

    // initalise system default state and database
    let state = web::Data::new(State {
        unauthenticated_servers: Default::default(),
        servers: Default::default(),
        requests: RwLock::new(HashMap::new()),
        base_url: domain,
        start_time: std::time::Instant::now(),
    });
    let database = web::Data::new(
        Database::new(db_url)
            .await
            .expect("a valid database connection"),
    );

    // begin listening for connections
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(database.clone())
            .service(endpoints::auth::register)
            .service(endpoints::upload::upload)
            .service(endpoints::websockets::websocket)
            .configure(endpoints::info::configure)
            .configure(endpoints::download::configure)
            .wrap(Logger::default())
    })
    .bind((host, port))?
    .run()
    .await
}
