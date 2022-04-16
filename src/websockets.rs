use std::time::Duration;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use actix_web_actors::ws::{self, CloseCode, CloseReason};
use log::{debug, error, info, trace};
use tokio::sync::mpsc;
use ws_com_framework::{error::ErrorKind, Message};

use crate::{
    db::{Database, DbBackend},
    ServerId, State,
};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum InternalComm {
    Authenticated,
    ShutDownComplete
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum WsState {
    Running,
    Stopping,
    Stopped,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum Authentication {
    Authenticated,
    Unauthenticated,
}

#[derive(Debug)]
pub struct WsHandler<T>
where
    T: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    /// messages sent here will be sent out to the connected agent
    tx: mpsc::Sender<Message>,
    /// a queue of messages to be sent to the connected agent
    rx: mpsc::Receiver<Message>,
    /// the send end of an internal message queue for various communications
    internal_tx: mpsc::Sender<InternalComm>,
    /// the receive end of an internal message queue for various communications
    internal_rx: mpsc::Receiver<InternalComm>,
    /// a pointer to the current state
    state: actix_web::web::Data<State>,
    /// the id of this server
    id: ServerId,
    /// a pointer to the database connection
    database: T,
    /// the current running state of the server
    ws_state: WsState,
    /// the current authentication state of the server
    authentication: Authentication,
}

impl<T> Actor for WsHandler<T>
where
    T: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.listen_for_response(ctx);
        self.handle_internal_comms(ctx);
    }

    fn stopping(&mut self, ctx: &mut Self::Context) -> actix::Running {
        trace!("got stopping request, current state {:?}", self.ws_state);
        match self.ws_state {
            WsState::Running => {
                trace!("beginning stopping operation");
                //ensure continuance while the context clears itself from the server list and shuts down
                self.ws_state = WsState::Stopping;

                let state = self.state.clone();
                let id = self.id;
                let i_tx = self.internal_tx.clone();
                let fut = async move {
                    state.servers.write().await.remove(&id);
                    if let Err(e) = i_tx.send(InternalComm::ShutDownComplete).await {
                        error!("unable to send internal message {:?}", e);
                    }
                };
                let fut = actix::fut::wrap_future::<_, Self>(fut);
                ctx.spawn(fut);

                actix::Running::Continue
            }
            WsState::Stopping => actix::Running::Continue,
            WsState::Stopped => actix::Running::Stop,
        }
    }
}

impl<T> WsHandler<T>
where
    T: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    fn listen_for_response(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_millis(50), |act, c| {
            if !matches!(act.ws_state, WsState::Running) {
                return; //In the process of asynchronously shutting down
            }

            macro_rules! close_connection {
                () => {{
                    c.close(None);
                    act.finished(c);
                }};
            }

            macro_rules! send_msg {
                ($msg:expr) => {{
                    let msg = $msg;
                    trace!("Sending message: {:?}", msg);
                    if let Ok(data) = TryInto::<Vec<u8>>::try_into(msg) {
                        c.write_raw(ws::Message::Binary(actix_web::web::Bytes::from(data)));
                    } else {
                        error!("failed to send message down websocket");
                    }
                }};
            }

            match act.rx.try_recv() {
                Ok(Message::Close) => close_connection!(),
                Ok(Message::Error(reason, kind)) => send_msg!(Message::Error(reason, kind)),
                Ok(msg) => send_msg!(msg),
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => close_connection!(),
            }
        });
    }

    fn handle_internal_comms(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_millis(1000), |act, _c| {
            if matches!(act.ws_state, WsState::Stopped) {
                return; //Prevent handling of internal messages when stopped
            }

            match act.internal_rx.try_recv() {
                Ok(InternalComm::Authenticated) => act.authentication = Authentication::Authenticated,
                Ok(InternalComm::ShutDownComplete) => act.ws_state = WsState::Stopped,
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => panic!("internal communication pipeline should never stop"),
            };
        });
    }
}

impl<T> StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsHandler<T>
where
    T: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    //XXX: include some way to identify which agent this is handling
    //XXX: should this be run in a ctx.run_interval?
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        macro_rules! close_unexpected_message {
            ($m:expr) => {{
                debug!("got unexpected message from agent: {:?}", $m);
                let error: Vec<u8> = Message::Error(
                    Some(String::from("Got unknown message")),
                    ErrorKind::Unknown,
                )
                .try_into()
                .expect("valid bytes");
                ctx.binary(actix_web::web::Bytes::from(error));
                ctx.close(Some(CloseReason {
                    code: CloseCode::Invalid,
                    description: Some(String::from("Got unknown message")),
                }));
                self.finished(ctx);
            }};
        }

        if !matches!(self.ws_state, WsState::Running) {
            return; //In the process of asynchronously shutting down
        }

        match item {
            Ok(ws::Message::Binary(msg)) => match Message::try_from(&msg[..]) {
                Ok(m) => {
                    trace!("Got binary message from peer: {m:?}");
                    match m {
                        Message::Ok => { /* Acknowledgement, do nothing */ }
                        Message::Error(resn, kind) => {
                            error!("Got error from agent: {:?}, {:?}", kind, resn)
                        }
                        Message::MetadataRes(share, upload_id) => {
                            let state = self.state.clone();
                            let fut = async move {
                                if let Some(s) = state.requests.write().await.remove(&upload_id) {
                                    //TODO
                                } else {
                                    trace!("request timed out before agent sent response");
                                }
                            };
                            let fut = actix::fut::wrap_future::<_, Self>(fut);
                            ctx.spawn(fut);
                        }
                        Message::AuthRes(public_id, passcode) => {
                            let database = self.database.clone();
                            let state = self.state.clone();
                            let tx = self.tx.clone();
                            let i_tx = self.internal_tx.clone();
                            let fut = async move {
                                match database.validate_server(&public_id, &passcode).await {
                                    Ok(true) => {
                                        let unauth_server = state
                                            .unauthenticated_servers
                                            .write()
                                            .await
                                            .remove(&public_id);
                                        match unauth_server {
                                            Some(s) => {
                                                state.servers.write().await.insert(public_id, s);
                                                if let Err(e) = tx.send(Message::Ok).await {
                                                    error!("unable to send OK auth response to peer {:?}", e);
                                                }
                                                if let Err(e) = i_tx.send(InternalComm::Authenticated).await {
                                                    error!("unable to send authentication msg internally {:?}", e);
                                                }
                                            },
                                            None => trace!("server disconnected before authentication could complete"),
                                        }
                                    }
                                    Ok(false) => {
                                        if let Err(e) = tx
                                            .send(Message::Error(
                                                Some(String::from("failed authentication")),
                                                ErrorKind::InvalidSession,
                                            ))
                                            .await
                                        {
                                            error!("unable to send auth failure req to peer due to error {:?}", e);
                                        }
                                        if let Err(e) = tx.send(Message::Close).await {
                                            error!("unable to close connection to peer {:?}", e);
                                        }
                                    }
                                    Err(e) => error!("database error: {}", e),
                                };
                            };
                            let fut = actix::fut::wrap_future::<_, Self>(fut);
                            ctx.spawn(fut);
                        }
                        _ => {
                            //Recieved unexpected message from agent
                            close_unexpected_message!(m);
                            self.finished(ctx);
                        }
                    }
                }
                Err(e) => error!("failed to decode binary message from peer {e:?}"),
            },
            Ok(ws::Message::Ping(msg)) => {
                ctx.write_raw(ws::Message::Pong(msg));
            }
            Ok(ws::Message::Pong(_)) => {
                trace!("recived pong message from peer"); //XXX find a way to send ping message reguararly
            }
            Ok(ws::Message::Close(reason)) => {
                info!("peer closing {reason:?}");
                self.finished(ctx);
            }
            Ok(msg) => {
                //Any other message is an error
                debug!("got unexpected message from peer: {msg:?}");
                close_unexpected_message!(msg);
                self.finished(ctx);
            }
            Err(e) => {
                error!("protocol error {:?}", e);
                self.finished(ctx);
            }
        }
    }
}

pub async fn __websocket<E>(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<State>,
    database: E,
    path: web::Path<ServerId>,
) -> Result<HttpResponse, actix_web::Error>
where
    E: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    let server_id = path.into_inner();

    let (tx, rx) = mpsc::channel(10);
    tx.send(Message::AuthReq(server_id)).await.unwrap();

    if state.servers.read().await.contains_key(&server_id) {
        return Ok(
            HttpResponse::Forbidden().body("another server is already authenticated with this id")
        );
    }

    let mut servers = state.unauthenticated_servers.write().await;
    servers.insert(server_id, tx.clone());

    let (i_tx, i_rx) = mpsc::channel(10);

    ws::start(
        WsHandler {
            tx,
            rx,
            state: state.clone(),
            database: database.clone(),
            id: server_id,
            ws_state: WsState::Running,
            authentication: Authentication::Unauthenticated,
            internal_tx: i_tx,
            internal_rx: i_rx,
        },
        &req,
        stream,
    )
}

#[get("/ws/{server_id}")]
pub async fn websocket(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<State>,
    database: web::Data<Database>,
    path: web::Path<ServerId>,
) -> impl Responder {
    __websocket(req, stream, state, database, path).await
}
