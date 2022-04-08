
// #![deny(
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

mod db;
mod auth;
mod websockets;
mod upload;
mod download;

use std::collections::HashMap;
use actix_web::{HttpServer, App, web::{self, Bytes}, HttpRequest, HttpResponse, get, post, error::PayloadError};
use db::{MockDb, DbBackend};
use diesel::{r2d2::{ConnectionManager, self}, SqliteConnection};
use tokio::sync::{mpsc, RwLock};
use ws_com_framework::{FileId, PublicId as ServerId, Passcode, Message};

type RequestId = u64;

pub struct State {
    unauthenticated_servers: RwLock<HashMap<ServerId, mpsc::Sender<Message>>>,
    servers: RwLock<HashMap<ServerId, mpsc::Sender<Message>>>,
    requests: RwLock<
                HashMap<
                    RequestId,
                    mpsc::Sender<Result<Bytes, PayloadError>>
                    >
                >,
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
    let database = web::Data::new(MockDb::new().await);
    HttpServer::new(move ||
        App::new()
            .app_data(state.clone())
            .app_data(database.clone())
            .service(websockets::websocket)
            .service(download::download)
            .service(upload::upload)
    )
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}