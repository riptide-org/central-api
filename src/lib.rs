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

///////// Endpoints/Features /////////
//TODO: update /info endpoints to return more information on individual nodes (include data on whether they're authenticated or not)
//TODO: add rate limiting for all endpoints - can be implemented as middleware

///////// Stretch Goals /////////
//TODO: allow some users to optionally cache the metadata/status response for a period of time, to reduce the number of requests
//TODO: allow some users to optionally cache the file upload itself for a period of time (e.g. 5 minutes), to reduce the number of requests
//TODO: add size/item limits for both of the above

///////// Refactoring /////////
//TODO: refactor return types into a global constant for the entire application
//TODO: relevant to above, refactor the metadata/status response generation from websockets into download/info
//XXX: refactor the `openapi.oas.yml` file to reduce duplication

#[macro_use]
extern crate diesel;

pub mod config;
pub mod db;
pub mod endpoints;
pub mod models;
#[cfg(not(tarpaulin_include))]
pub mod schema;
pub mod timelocked_hashmap;
pub mod util;

use actix_extensible_rate_limit::{
    backend::{memory::InMemoryBackend, SimpleInputFunctionBuilder},
    RateLimiter,
};
use actix_web::{
    error::PayloadError,
    middleware::Logger,
    web::{Bytes, Data},
    App, HttpServer,
};
use config::Config;
use db::{Database, DbBackend};
use endpoints::websockets::InternalComm as WsInternalComm;
use log::info;
use std::{collections::HashMap, time::Duration};
use timelocked_hashmap::TimedHashMap;
use tokio::sync::{mpsc, RwLock};
use ws_com_framework::PublicId as ServerId;

type RequestId = u64;

/// State holds information about all current active connections and nodes
#[derive(Debug)]
pub struct State {
    /// Websockets that are connected, but have not yet completed authentication
    pub unauthenticated_servers: RwLock<TimedHashMap<ServerId, mpsc::Sender<WsInternalComm>>>,
    /// Connected websockets that are valid, and have responded to a ping within the last X seconds
    pub servers: RwLock<HashMap<ServerId, mpsc::Sender<WsInternalComm>>>,
    /// Actively waiting requests that need an agent to respond - that also haven't timed out yet
    pub requests: RwLock<TimedHashMap<RequestId, mpsc::Sender<Result<Bytes, PayloadError>>>>,
    /// The base URL of this server
    pub base_url: String,
    /// The instant that the server started
    pub start_time: std::time::Instant,
}

#[doc(hidden)]
pub async fn start(config: Config, state: Data<State>) -> Result<(), Box<dyn std::error::Error>> {
    let config = Data::new(config);

    let database = Data::new(
        Database::new(&config.db_url)
            .await
            .expect("a valid database connection"),
    );

    // start a monitoring task to remove expired entries from the state
    let watcher_handle = util::start_watcher(state.clone(), config.auth_timeout_seconds);

    // begin listening for connections
    let server_config = config.clone();
    let mut server_handle = HttpServer::new(move || {
        // create a simple ratelimiter
        let ratelimiter_backend = InMemoryBackend::builder().build();
        let ratelimiter_input = SimpleInputFunctionBuilder::new(Duration::from_secs(60), 10)
            .real_ip_key()
            .build();
        let ratelimiter_middleware =
            RateLimiter::builder(ratelimiter_backend, ratelimiter_input).build();

        App::new()
            .app_data(state.clone())
            .app_data(database.clone())
            .app_data(server_config.clone())
            .service(endpoints::auth::register)
            .service(endpoints::upload::upload)
            .service(endpoints::websockets::websocket)
            .configure(endpoints::info::configure)
            .configure(endpoints::download::configure)
            .wrap(ratelimiter_middleware)
            .wrap(Logger::default())
    })
    .listen(config.listener.try_clone().unwrap())?
    .run();

    // pin values in place for validity over many loops
    let mut watcher_handle = Box::pin(watcher_handle);

    loop {
        tokio::select! {
            biased;

            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT, shutting down");
                break;
            }

            _ = &mut watcher_handle => {
                info!("Watcher task has exited, shutting down");
                break;
            }

            _ = &mut server_handle => {
                info!("Server task has exited, shutting down");
                break;
            }
        }
    }

    watcher_handle.abort();

    Ok(())
}
