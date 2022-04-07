use std::collections::HashMap;
use actix_web::{HttpServer, App, web::{self, Bytes}, HttpRequest, HttpResponse, get, post, error::PayloadError};
use db::MockDB;
use diesel::{r2d2::{ConnectionManager, self}, SqliteConnection};
use futures::StreamExt;
use log::{trace, warn, info, error};
use rand::Rng;
use tokio::sync::{mpsc, RwLock};
use websockets::websocket;
use ws_com_framework::{FileId, PublicId as ServerId, Passcode, Message};

mod db;
mod websockets;

type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

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

/// Download a file from a client
#[get("/download/{server_id}/{file_id}")]
async fn download(req: HttpRequest, state: web::Data<State>, path: web::Path<(ServerId, FileId)>) -> HttpResponse {
    let (server_id, file_id) = path.into_inner();

    //Check server is online
    let reader = state.servers.read().await;
    let server_online = reader.contains_key(&server_id); //Duplicate req #cd
    if server_online {
        let (tx, mut rx) = mpsc::channel(100);

        let download_id = rand::thread_rng().gen();

        //Create a valid upload job
        state.requests.write().await.insert(download_id, tx);

        //Acquire channel to WS, and send upload req. to server
        let msg = format!("{}/upload/{}", state.base_url, download_id);
        let connected_servers = state.servers.read().await;
        let uploader_ws = connected_servers.get(&server_id).unwrap(); //Duplicate req #cd
        uploader_ws.send(Message::UploadTo(file_id, msg)).await.unwrap();

        let payload = async_stream::stream! {
            while let Some(v) = rx.recv().await {
                yield v;
            }
        };

        //create a streaming response
        HttpResponse::Ok()
            .content_type("text/html")
            .streaming(payload)
    } else {
        trace!("client attempted to request file {} from {:?}, but that server isn't connected", file_id, server_id);
        HttpResponse::NotFound()
            .content_type("text/html")
            .body("requested resource not found, the server may not be connected")
    }
}

/// Upload a file or metadata to a waiting client
#[post("/upload/{upload_id}")]
async fn upload(req: HttpRequest, mut payload: web::Payload, state: web::Data<State>, path: web::Path<ServerId>) -> HttpResponse {
    let upload_id = path.into_inner();

    //Get uploadee channel
    let mut sender_store = state.requests.write().await;
    println!("the upload id is: {upload_id:?}");
    println!("available keys are: {:?}", sender_store.keys());
    let sender = sender_store.remove(&upload_id).unwrap();

    //XXX timeout?
    while let Some(chk) = payload.next().await {
        if let Err(e) = sender.send(chk).await {
            error!("problem sending payload {:?}", e);
            return HttpResponse::InternalServerError()
                .body("upload failed");
        };
    };

    HttpResponse::Ok()
        .body("succesfully uploaded")
}

/// Attempt to register a new webserver with the api
#[post("/register")]
pub async fn register() -> HttpResponse {
    todo!()
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
    let database = MockDB {};
    HttpServer::new(move ||
        App::new()
            .app_data(state.clone())
            .app_data(database.clone())
            .service(websocket)
            .service(download)
            .service(upload)
    )
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}