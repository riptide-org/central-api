use std::time::Duration;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use log::{info, warn, error};
use tokio::sync::mpsc;
use ws_com_framework::Message;

use crate::{ServerId, State};

#[derive(Debug)]
pub struct WsHandler {
    rcv: mpsc::Receiver<Message>,
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
            macro_rules! close_connection {
                () => {
                    {
                        c.close(None);
                        act.finished(c);
                    }
                };
            }

            macro_rules! send_msg {
                ($msg:expr) => {
                    {
                        if let Ok(data) = TryInto::<Vec<u8>>::try_into($msg) {
                            c.write_raw(ws::Message::Binary(actix_web::web::Bytes::from(data)));
                        } else {
                            error!("failed to send message down websocket");
                        }
                    }
                }
            }

            match act.rcv.try_recv() {
                Ok(Message::Close) => close_connection!(),
                Ok(Message::Error(reason, end_connection, kind)) => {
                    send_msg!(Message::Error(reason, end_connection, kind));
                    if Into::<bool>::into(end_connection) {
                        close_connection!()
                    }
                },
                Ok(msg) => send_msg!(msg),
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
                info!("received request from user {:?}", msg);
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
    tx.send(Message::AuthReq(server_id)).await.unwrap();

    if state.servers.read().await.contains_key(&server_id) {
        return Ok(HttpResponse::Forbidden().body("another server is already authenticated with this id"))
    }

    let mut servers = state.unauthenticated_servers.write().await;
    servers.insert(server_id, tx);

    ws::start(WsHandler { rcv: rx }, &req, stream)
}