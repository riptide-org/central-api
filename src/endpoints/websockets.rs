//! Websocket handlers and processing methods for the central API.

use std::time::Duration;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{
    error::PayloadError,
    get,
    web::{self, Bytes, Data},
    HttpRequest, HttpResponse, Responder,
};
use actix_web_actors::ws::{self, CloseCode, CloseReason};
use log::{debug, error, info, trace};
use tokio::sync::mpsc;
use ws_com_framework::{error::ErrorKind, Message};

use crate::{
    config::Config,
    db::{Database, DbBackend},
    ServerId, State,
};

/// Internal communication between an agent and the central-api
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, PartialEq)]
pub enum InternalComm {
    /// Client has successfully authenticated
    Authenticated,
    /// The central-api has shutdown the connection to this agent fully
    ShutDownComplete,
    /// A message should be sent to this agent
    SendMessage(Message),
    /// This connection should be closed
    CloseConnection,
    /// (TEST ONLY): A message has been received from the agent
    #[allow(dead_code)]
    #[cfg(test)]
    ReceiveMessage(actix_web_actors::ws::Message),
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

/// The websocket handler for handling connection over websockets from individual agents
#[derive(Debug)]
pub struct WsHandler<T>
where
    T: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    /// the send end of an internal message queue for various communications
    internal_tx: mpsc::Sender<InternalComm>,
    /// the receive end of an internal message queue for various communications
    internal_rx: mpsc::Receiver<InternalComm>,
    /// a pointer to the current state
    state: Data<State>,
    /// a pointer to the current config
    config: Data<Config>,
    /// the id of this server
    id: ServerId,
    /// a pointer to the database connection
    database: T,
    /// the current running state of the server
    ws_state: WsState,
    /// the current authentication state of the server
    authentication: Authentication,
    /// a tracker for the current ping status
    pinger: u64,
}

impl<T> Actor for WsHandler<T>
where
    T: DbBackend + 'static + Unpin + Send + Sync + Clone,
{
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.internal_comms(ctx);
        self.ping_pong(ctx);
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
    fn internal_comms(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        // Note that we are only draining one action from the queue each time this function runs
        // this means a connected agent will get at *most* 20 actions per second.
        // It could be an interesting activity to dynamically drain more elements from the queue if many
        // are waiting. Could track number of elements in queue with a simple counter.
        ctx.run_interval(Duration::from_millis(50), |act, c| {
            macro_rules! close_connection {
                () => {{
                    c.close(None);
                    act.finished(c);
                }};
            }

            if matches!(act.ws_state, WsState::Stopped) {
                return; //Prevent handling of internal messages when stopped
            }

            match act.internal_rx.try_recv() {
                Ok(InternalComm::Authenticated) => {
                    act.authentication = Authentication::Authenticated
                }
                Ok(InternalComm::CloseConnection) => close_connection!(),
                Ok(InternalComm::ShutDownComplete) => {
                    act.ws_state = WsState::Stopped;
                    // act.finished(c)
                }
                Ok(InternalComm::SendMessage(msg)) => {
                    trace!("Sending message: {:?}", msg);
                    if let Ok(data) = TryInto::<Vec<u8>>::try_into(msg) {
                        c.write_raw(ws::Message::Binary(actix_web::web::Bytes::from(data)));
                    } else {
                        error!("failed to send message down websocket");
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    error!("internal communication pipeline should never stop");
                    close_connection!();
                }
                #[cfg(test)]
                Ok(InternalComm::ReceiveMessage(msg)) => {
                    act.handle(Ok(msg), c);
                }
            }
        });
    }

    fn ping_pong(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_secs(self.config.ping_interval), |act, c| {
            if matches!(act.ws_state, WsState::Stopped) {
                return; //Prevent handling of internal messages when stopped
            }
            c.ping(&act.pinger.to_be_bytes());
            act.pinger += 1; //TODO; check received in order
        });
    }
}

impl<T> StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsHandler<T>
where
    T: DbBackend + Unpin + Send + Sync + Clone,
{
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        macro_rules! close_unexpected_message {
            ($m:expr) => {{
                debug!("got unexpected message from agent: {:?}", $m);
                let error: Vec<u8> = Message::Error {
                    kind: ErrorKind::Unknown,
                    reason: Some(String::from("Got unknown message")),
                }
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
                        Message::Error { kind, reason } => {
                            error!("Got error message from peer: {kind:?} {reason:?}");
                            if matches!(kind, ErrorKind::FileDoesntExist) {
                                if let Some(id) = reason {
                                    let state = self.state.clone();
                                    let id = id.parse::<u64>().expect("valid id");
                                    let fut = async move {
                                        state.requests.write().await.remove(&id);
                                    };
                                    let fut = actix::fut::wrap_future::<_, Self>(fut);
                                    ctx.spawn(fut);
                                }
                            } else {
                                error!("Got error from agent: {:?}, {:?}", kind, reason)
                            }
                        }
                        Message::MetadataRes {
                            file_id,
                            exp,
                            crt,
                            file_size,
                            username,
                            file_name,
                            upload_id,
                        } => {
                            let state = self.state.clone();
                            let fut = async move {
                                if let Some(s) = state.requests.write().await.remove(&upload_id) {
                                    let formatted_body = format!(
                                        "{{
                                        \"file_id\":{},
                                        \"exp\": {},
                                        \"crt\": {},
                                        \"file_size\": {},
                                        \"username\": \"{}\",
                                        \"file_name\": \"{}\"
                                    }}",
                                        file_id, exp, crt, file_size, username, file_name
                                    );
                                    if let Err(e) = s
                                        .send(Ok(actix_web::web::Bytes::from(formatted_body)))
                                        .await
                                    {
                                        error!("failed to send metadata response to peer, they may have timed out waiting: `{}`", e);
                                    };
                                } else {
                                    error!("request timed out before agent sent response");
                                }
                            };
                            let fut = actix::fut::wrap_future::<_, Self>(fut);
                            ctx.spawn(fut);
                        }
                        Message::AuthRes {
                            public_id,
                            passcode,
                        } => {
                            let database = self.database.clone();
                            let state = self.state.clone();
                            let tx = self.internal_tx.clone();
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
                                                if let Err(e) = tx.send(InternalComm::SendMessage(Message::Ok)).await {
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
                                            .send(InternalComm::SendMessage(Message::Error {
                                                reason: Some(String::from("failed authentication")),
                                                kind: ErrorKind::InvalidSession,
                                            }))
                                            .await
                                        {
                                            error!("unable to send auth failure req to peer due to error {:?}", e);
                                        }
                                        if let Err(e) = tx.send(InternalComm::CloseConnection).await
                                        {
                                            error!("unable to close connection to peer {:?}", e);
                                        }
                                    }
                                    Err(e) => error!("database error: {}", e),
                                };
                            };
                            let fut = actix::fut::wrap_future::<_, Self>(fut);
                            ctx.spawn(fut);
                        }
                        Message::StatusRes {
                            public_id,
                            ready,
                            uptime,
                            upload_id,
                            message,
                        } => {
                            let state = self.state.clone();
                            let fut = async move {
                                if let Some(s) = state.requests.write().await.remove(&upload_id) {
                                    let formatted_body = format!(
                                        "{{
                                        \"public_id\": \"{}\",
                                        \"ready\": {},
                                        \"uptime\": {},
                                        \"message\": \"{}\"
                                    }}",
                                        public_id,
                                        ready,
                                        uptime,
                                        message.unwrap_or_else(|| String::from(""))
                                    );
                                    if let Err(e) = s
                                        .send(Ok(actix_web::web::Bytes::from(formatted_body)))
                                        .await
                                    {
                                        error!("failed to send status response to peer, they may have timed out waiting: `{}`", e);
                                    };
                                } else {
                                    error!("request timed out before agent sent response");
                                }
                            };
                            let fut = actix::fut::wrap_future::<_, Self>(fut);
                            ctx.spawn(fut);
                        }
                        _ => {
                            //Received unexpected message from agent
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
            Ok(ws::Message::Pong(pong)) => {
                trace!("Got pong from peer: {:?}", pong);
                if pong.as_ref() != (self.pinger - 1).to_be_bytes() {
                    //pong failed
                    trace!("peer send pong response too slowly");

                    let error: Vec<u8> = Message::Error {
                        kind: ErrorKind::Unknown,
                        reason: Some(String::from("failed to respond to ping")),
                    }
                    .try_into()
                    .expect("valid bytes");
                    ctx.binary(error);
                    ctx.close(Some(CloseReason {
                        code: CloseCode::Protocol,
                        description: Some(String::from("failed to respond to ping")),
                    }));
                    self.finished(ctx);
                }

                //every 10 pings, update last seen //XXX: test
                if self.pinger % 10 == 0 {
                    let database = self.database.clone();
                    let id = self.id;
                    let fut = async move {
                        if let Err(e) = database.update_last_seen(&id).await {
                            error!(
                                "failed to update last seen for peer {} due to error: {}",
                                id, e
                            );
                        }
                    };

                    let fut = actix::fut::wrap_future::<_, Self>(fut);
                    ctx.spawn(fut);
                }
            }
            Ok(ws::Message::Close(reason)) => {
                info!("peer closing {reason:?}");
                self.finished(ctx);
            }
            Ok(msg) => {
                //Any other message is an error
                error!("got unexpected message from peer: {msg:?}");
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

async fn __websocket<E, T>(
    req: HttpRequest,
    stream: T,
    state: Data<State>,
    config: Data<Config>,
    database: E,
    server_id: ServerId,
) -> Result<HttpResponse, actix_web::Error>
where
    E: DbBackend + 'static + Unpin + Send + Sync + Clone,
    T: futures::Stream<Item = Result<Bytes, PayloadError>> + 'static,
{
    let (tx, rx) = mpsc::channel(100);
    tx.send(InternalComm::SendMessage(Message::AuthReq {
        public_id: server_id,
    }))
    .await
    .unwrap();

    if state.servers.read().await.contains_key(&server_id) {
        return Ok(
            HttpResponse::Forbidden().body("another server is already authenticated with this id")
        );
    }

    state
        .unauthenticated_servers
        .write()
        .await
        .insert(server_id, tx.clone());

    ws::start(
        WsHandler {
            state: state.clone(),
            database: database.clone(),
            id: server_id,
            ws_state: WsState::Running,
            authentication: Authentication::Unauthenticated,
            pinger: 0,
            internal_tx: tx,
            internal_rx: rx,
            config,
        },
        &req,
        stream,
    )
}

#[get("/ws/{server_id}")]
pub async fn websocket(
    req: HttpRequest,
    stream: web::Payload,
    state: Data<State>,
    config: Data<Config>,
    database: Data<Database>,
    path: web::Path<ServerId>,
) -> impl Responder {
    let server_id = path.into_inner();
    __websocket(req, stream, state, config, database, server_id).await
}
