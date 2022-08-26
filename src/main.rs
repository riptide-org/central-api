//! The central api which handles websockets and file streams between uploaders and clients

#![warn(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    // clippy::missing_docs_in_private_items, //TODO
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications,
    deprecated
)]

//TODO: if a request is present for more than 50 seconds, time it out and return an error - they shouldn't hang around in the
// state.

//TODO: remove unauthenticated agents after 5 minutes

//TODO: update the last_seen timer whenever an agent authenticates.
//TODO: force agents to reauthenticate periodically
//TODO: expand configuration to cover all attached variables, in a separate module and pass that throughout the application
//TODO: refactor return types into a global constant for the entire application
//TODO: relevant to above, refactor the metadata/status response generation from websockets into download/info
//TODO: refactor the integration tests in websockets.rs into a file in tests/integration.rs
//TODO: add more unit tests generally speaking, and add integration tests for information endpoints, and metadata
//TODO: add test to ensure that the agent is removed from the state when it disconnects
//TODO: write tests to cover error states
//TODO: refactor the openapi.oas.yml file to reduce duplication
//TODO: update /info endpoints to return more information on individual nodes (include data on whether they're authenticated or not)
//TODO: allow some users to optionally cache the metadata/status response for a period of time, to reduce the number of requests
//TODO: allow some users to optionally cache the file upload itself for a period of time (e.g. 5 minutes), to reduce the number of requests
//TODO: add size/item limits for both of the above
//TODO: add rate limiting for all endpoints - can be implemented as middleware

#[macro_use]
extern crate diesel;

mod db;
mod endpoints;
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
use endpoints::websockets::InternalComm as WsInternalComm;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
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
