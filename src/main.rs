// #![warn(
//     missing_docs,
//     missing_debug_implementations,
//     missing_copy_implementations,
//     trivial_casts,
//     trivial_numeric_casts,
//     unsafe_code,
//     unstable_features,
//     unused_import_braces,
//     unused_qualifications,
//     deprecated
// )]

#[macro_use]
extern crate diesel;

mod auth;
mod db;
mod download;
mod error;
mod models;
mod schema;
mod upload;
mod websockets;

use actix_web::{
    error::PayloadError,
    get, post,
    web::{self, Bytes},
    App, HttpRequest, HttpResponse, HttpServer,
};
use db::{Database, DbBackend};
use diesel::{
    r2d2::{self, ConnectionManager},
    SqliteConnection,
};
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use ws_com_framework::{FileId, Message, Passcode, PublicId as ServerId};

type RequestId = u64;

pub struct State {
    unauthenticated_servers: RwLock<HashMap<ServerId, mpsc::Sender<Message>>>,
    servers: RwLock<HashMap<ServerId, mpsc::Sender<Message>>>,
    requests: RwLock<HashMap<RequestId, mpsc::Sender<Result<Bytes, PayloadError>>>>,
    base_url: String,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();
    let state = web::Data::new(State {
        unauthenticated_servers: Default::default(),
        servers: Default::default(),
        requests: RwLock::new(HashMap::new()),
        base_url: "http://127.0.0.1:8080".into(),
    });
    let database = web::Data::new(Database::new().await.expect("a valid database connection"));
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(database.clone())
            .service(auth::register)
            .service(websockets::websocket)
            .service(download::download)
            .service(upload::upload)
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
