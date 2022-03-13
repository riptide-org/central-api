use std::{time::Duration, collections::HashMap, sync::{atomic::{AtomicUsize, Ordering}}};
use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{HttpServer, App, web::{self, Bytes}, HttpRequest, HttpResponse, get, post, error::PayloadError};
use actix_web_actors::ws;
use diesel::{r2d2::{ConnectionManager, self}, SqliteConnection};
use futures::StreamExt;
use log::{trace, warn, info, error};
use tokio::sync::{mpsc, RwLock};

type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

struct State {
    servers: RwLock<HashMap<String, mpsc::Sender<WsAction>>>,
    requests: RwLock<
                HashMap<
                    String,
                    mpsc::Sender<Result<Bytes, PayloadError>>
                    >
                >,
    counter: AtomicUsize,
    base_url: String,
}

struct WsHandler {
    rcv: mpsc::Receiver<WsAction>,
}

impl Actor for WsHandler {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.listen_for_response(ctx);
    }
}

//XXX use something like a protobuf to handle sending the data over websockets, this way we get better data compression
impl WsHandler {
    fn listen_for_response(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_millis(50), |act, c| {
            match act.rcv.try_recv() {
                Ok(WsAction::UploadTo(msg, id)) => c.write_raw(ws::Message::Text(format!("url {msg}\nfile {id}").into())),
                Ok(WsAction::ServerId(id)) => c.write_raw(ws::Message::Text(format!("your server id is {id}").into())),
                Err(mpsc::error::TryRecvError::Empty) => (),
                Err(mpsc::error::TryRecvError::Disconnected) => act.finished(c),
            }
        });
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsHandler {
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match item {
            Ok(msg) => {
                info!(
                    "received request from user and forward {:?}", msg
                );
            },
            Err(e) => {
                warn!(
                    "received an error from client {:?}\ngoing to stop connection",
                    e
                );
                self.finished(ctx);
            }
        }
    }
}

#[derive(Debug)]
enum WsAction {
    UploadTo(String, String), //url - file_id
    ServerId(usize),
}

#[get("/websocket")]
async fn websocket(req: HttpRequest, stream: web::Payload, state: web::Data<State>) -> Result<HttpResponse, actix_web::Error> {
    let (tx, rx) = mpsc::channel(10);
    let server_id = state.counter.fetch_add(1, Ordering::Relaxed);
    tx.send(WsAction::ServerId(server_id)).await.unwrap();

    let mut servers = state.servers.write().await;
    servers.insert(format!("{}", server_id), tx);

    ws::start(WsHandler {
        rcv: rx,
    }, &req, stream)
}

#[get("/download/{server_id}/{file_id}")]
async fn download(req: HttpRequest, state: web::Data<State>) -> HttpResponse {
    let server_id: &str = req.match_info().get("server_id").unwrap();
    let file_id: &str = req.match_info().get("file_id").unwrap();

    //Check server is online
    let reader = state.servers.read().await;
    let server_online = reader.contains_key(server_id); //Duplicate req #cd
    if server_online {
        //Create
        let download_id = state.counter.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel(100);

        //Create a valid upload job
        state.requests.write().await.insert(format!("{}", download_id), tx);

        //Acquire channel to WS, and send upload req. to server
        let msg = format!("{}/upload/{}", state.base_url, download_id);
        let connected_servers = state.servers.read().await;
        let uploader_ws = connected_servers.get(server_id).unwrap(); //Duplicate req #cd
        uploader_ws.send(WsAction::UploadTo(msg, file_id.to_string())).await.unwrap();

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
        trace!("client attempted to request file {} from {}, but that server isn't connected", file_id, server_id);
        HttpResponse::NotFound()
            .content_type("text/html")
            .body("requested resource not found, the server may not be connected")
    }
}

#[post("/upload/{upload_id}")]
async fn upload(req: HttpRequest, mut payload: web::Payload, state: web::Data<State>) -> HttpResponse {
    let upload_id: &str = req.match_info().get("upload_id").unwrap();

    //Get uploadee channel
    let mut sender_store = state.requests.write().await;
    println!("the upload id is: {upload_id}");
    println!("available keys are: {:?}", sender_store.keys());
    let sender = sender_store.remove(upload_id).unwrap();

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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let state = web::Data::new(State {
        servers: Default::default(),
        requests: RwLock::new(HashMap::new()),
        counter: AtomicUsize::new(0),
        base_url: "http://127.0.0.1:8080".into(),
    });
    HttpServer::new(move ||
        App::new()
            .app_data(state.clone())
            .service(websocket)
            .service(download)
            .service(upload)
    )
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}