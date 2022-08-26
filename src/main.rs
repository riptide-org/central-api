//! The central api which handles websockets and file streams between uploaders and clients

#![warn(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications,
    // deprecated
)]

#[macro_use]
extern crate diesel;

mod auth;
mod db;
mod download;
mod error;
mod models;
#[cfg(not(tarpaulin_include))]
mod schema;
mod upload;
mod websockets;

use actix_web::{
    error::PayloadError,
    web::{self, Bytes},
    App, HttpServer, middleware::Logger,
};
use db::{Database, DbBackend};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use ws_com_framework::PublicId as ServerId;
use websockets::InternalComm as WsInternalComm;
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
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();

    // setup for ssl encryption
    let mut builder =
        SslAcceptor::mozilla_intermediate(SslMethod::tls()).expect("a valid ssl intermediate");
    builder
        .set_private_key_file("certs/key.pem", SslFiletype::PEM)
        .expect("a valid private key");
    builder
        .set_certificate_chain_file("certs/cert.pem")
        .expect("valid cert pem");

    // initalise system default state and database
    let state = web::Data::new(State {
        unauthenticated_servers: Default::default(),
        servers: Default::default(),
        requests: RwLock::new(HashMap::new()),
        base_url: "https://127.0.0.1:8080".into(), //readonly //XXX: pull from env
    });
    let database_url: String = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let database = web::Data::new(Database::new(database_url).await.expect("a valid database connection"));

    // begin listening for connections
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(database.clone())
            .service(auth::register)
            .service(websockets::websocket)
            .service(download::metadata)
            .service(download::download)
            .service(upload::upload)
            .wrap(Logger::default())
    })
    .bind_openssl(("127.0.0.1", 8080), builder)?
    .run()
    .await
}
