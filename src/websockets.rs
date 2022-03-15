use std::time::Duration;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_web_actors::ws::{self, CloseReason};
use log::{info, warn};
use tokio::sync::mpsc;

use crate::{FileId, ServerId, State, db::{self, DbBackend}};

#[derive(Debug)]
pub enum WsAction {
    /// Upload the file contents to the provided url
    UploadTo(String, FileId),
    /// Request a metadata file upload to the provided url
    UploadMetadata(String, FileId),
    /// Flush and close the connection to the websocket,
    CloseConnection(Option<CloseReason>),
    /// Request authentication for the provided ServerId, the client should
    /// respond quickly or it wil be disconnected.
    RequestAuthentication(ServerId),
    /// The server should respond with it's current readiness
    RequestStatus,
}

#[derive(Debug)]
pub struct WsHandler {
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
                Ok(WsAction::UploadTo(msg, id)) => {
                    c.write_raw(ws::Message::Text(format!("url {msg}\nfile {id}").into()))
                }
                Ok(WsAction::UploadMetadata(msg, id)) => {} //TODO
                Ok(WsAction::RequestStatus) => {}           //TODO
                Ok(WsAction::RequestAuthentication(server_id)) => {} //TODO
                Ok(WsAction::CloseConnection(rsn)) => {
                    c.close(rsn);
                    act.finished(c);
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => act.finished(c),
            }
        });
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsHandler {
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match item {
            Ok(msg) => {
                info!("received request from user and forward {:?}", msg);
            }
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

#[get("/ws/{server_id}")]
pub async fn websocket(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<State>,
    path: web::Path<ServerId>,
) -> Result<HttpResponse, actix_web::Error> {
    let server_id = path.into_inner();


    let (tx, rx) = mpsc::channel(10);
    tx.send(WsAction::RequestAuthentication(server_id)).await.unwrap();

    if state.servers.read().await.contains_key(&server_id) {
        return Ok(HttpResponse::Forbidden().body("another server is already authenticated with this id"))
    }

    let mut servers = state.unauthenticated_servers.write().await;
    servers.insert(server_id, tx);

    ws::start(WsHandler { rcv: rx }, &req, stream)
}