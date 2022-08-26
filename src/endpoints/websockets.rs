use std::time::Duration;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{
    error::PayloadError,
    get,
    web::{self, Bytes},
    HttpRequest, HttpResponse, Responder,
};
use actix_web_actors::ws::{self, CloseCode, CloseReason};
use log::{debug, error, info, trace};
use tokio::sync::mpsc;
use ws_com_framework::{error::ErrorKind, Message};

use crate::{
    db::{Database, DbBackend},
    ServerId, State,
};

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, PartialEq)]
pub enum InternalComm {
    Authenticated,
    ShutDownComplete,
    SendMessage(Message),
    CloseConnection,
    #[allow(dead_code)]
    #[cfg(test)]
    RecieveMessage(actix_web_actors::ws::Message),
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
                Ok(InternalComm::ShutDownComplete) => act.ws_state = WsState::Stopped,
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
                Ok(InternalComm::RecieveMessage(msg)) => {
                    act.handle(Ok(msg), c);
                }
            }
        });
    }

    fn ping_pong(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_secs(20), |act, c| {
            c.ping(&act.pinger.to_be_bytes());
            act.pinger += 1;
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
                            error!("Got error from agent: {:?}, {:?}", kind, reason)
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
                trace!("received pong message from peer");
                if pong.as_ref() != (self.pinger - 1).to_be_bytes() {
                    //pong failed
                    error!("peer send pong response too slowly");
                    self.finished(ctx);
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

pub async fn __websocket<E, T>(
    req: HttpRequest,
    stream: T,
    state: web::Data<State>,
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
    let server_id = path.into_inner();
    __websocket(req, stream, state, database, server_id).await
}

#[cfg(test)]
#[cfg(not(tarpaulin_include))]
mod test {
    use std::{collections::HashMap, sync::Once, task::Poll, thread::JoinHandle};

    use actix_web::{
        error::PayloadError,
        middleware::Logger,
        web::{Bytes, Data},
        App, HttpServer,
    };
    use futures::Stream;
    use log::{error, info};
    use ntest::timeout;
    use serde::{Deserialize, Serialize};
    use tokio::sync::{mpsc::UnboundedReceiver, oneshot, RwLock};
    use tungstenite::Message;
    use ws_com_framework::error::ErrorKind;

    use crate::{
        db::{Database, DbBackend},
        State,
    };

    static INIT: Once = Once::new();

    fn init_logger() {
        INIT.call_once(|| {
            pretty_env_logger::init();
        });
    }

    #[derive(Clone, Serialize, Deserialize, Debug)]
    struct AuthToken {
        pub public_id: u64,
        pub passcode: String,
    }

    struct MockStreamer {
        rx: UnboundedReceiver<Vec<u8>>,
    }

    impl Stream for MockStreamer {
        type Item = Result<Bytes, PayloadError>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(x)) => Poll::Ready(Some(Ok(Bytes::from_iter(x)))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    fn find_open_port() -> std::net::TcpListener {
        for port in 1025..65535 {
            if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
                return l;
            }
        }
        panic!("no open ports found");
    }

    async fn create_server(
        database_url: String,
        port: std::net::TcpListener,
    ) -> (
        Data<Database>,
        Data<State>,
        JoinHandle<()>,
        oneshot::Sender<()>,
    ) {
        let state = Data::new(State {
            unauthenticated_servers: Default::default(),
            servers: Default::default(),
            requests: RwLock::new(HashMap::new()),
            base_url: "https://localhost:8080".into(),
            start_time: std::time::Instant::now(),
        });

        let db = Data::new(
            Database::new(database_url)
                .await
                .expect("a valid database connection"),
        );

        let server_db = db.clone();
        let server_state = state.clone();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let mut server = HttpServer::new(move || {
                App::new()
                    .app_data(server_db.clone())
                    .app_data(server_state.clone())
                    .service(crate::endpoints::auth::register)
                    .service(crate::endpoints::upload::upload)
                    .service(crate::endpoints::websockets::websocket)
                    .configure(crate::endpoints::info::configure)
                    .configure(crate::endpoints::download::configure)
                    .wrap(Logger::default())
            })
            .listen(port)
            .unwrap()
            .run();
            rt.block_on(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut rx => {
                            break;
                        }
                        _ = &mut server => {
                            break;
                        }
                    }
                }
            });
        });

        (db, state, handle, tx)
    }

    /// Test the websockets responses to various inputs
    #[actix_web::test]
    #[timeout(20_000)]
    async fn test_websocket_auth() {
        init_logger();
        let database_url = String::from("./test-db-test-websocket-auth.db");
        let port = find_open_port();
        let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
        let (db, state, handle, tx) = create_server(database_url.clone(), port).await;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // make POST request to http endpoint at /register to get a token
        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/register", address))
            .send()
            .await
            .unwrap();

        assert!(res.status().is_success());
        let token: AuthToken = res.json().await.unwrap();

        //validate that token is in the database
        assert!(db.contains_entry(&token.public_id).await.unwrap());

        // create a websocket connection to the server
        let (mut socket, _) =
            tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap();

        let msg = socket.read_message().unwrap();
        let expected_bytes: Vec<u8> = ws_com_framework::Message::AuthReq {
            public_id: token.public_id,
        }
        .try_into()
        .unwrap();
        assert_eq!(msg.into_data(), expected_bytes);

        // check that the state contains us as an entry in unauthenticated_servers
        assert!(state
            .unauthenticated_servers
            .read()
            .await
            .contains_key(&token.public_id));

        // send auth message to the server
        socket
            .write_message(Message::Binary(
                ws_com_framework::Message::AuthRes {
                    public_id: token.public_id,
                    passcode: token.passcode.into(),
                }
                .try_into()
                .unwrap(),
            ))
            .unwrap();

        // expect OK response
        let msg = socket.read_message().unwrap();
        let expected_bytes: Vec<u8> = ws_com_framework::Message::Ok.try_into().unwrap();
        assert_eq!(msg.into_data(), expected_bytes);

        // check that the state no longer contains us as an entry in unauthenticated_servers
        assert!(!state
            .unauthenticated_servers
            .read()
            .await
            .contains_key(&token.public_id));

        // check that the state contains us as an entry in servers
        assert!(state.servers.read().await.contains_key(&token.public_id));

        // test ping/pong
        socket.write_message(Message::Ping(vec![1, 2, 3])).unwrap();
        let msg = socket.read_message().unwrap();
        assert_eq!(msg, Message::Pong(vec![1, 2, 3]));

        // test close
        socket.write_message(Message::Close(None)).unwrap();

        //kill the std thread without waiting
        if let Err(e) = tx.send(()) {
            error!("{:?}", e);
        }
        handle.join().unwrap();

        // remove database file
        std::fs::remove_file(database_url).unwrap();
    }

    #[actix_web::test]
    #[timeout(20_000)]
    async fn test_websocket_auth_failure() {
        init_logger();
        let database_url = String::from("./test-db-test-websocket-auth-failure.db");
        let port = find_open_port();
        let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
        let (_, _, handle, tx) = create_server(database_url.clone(), port).await;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // make POST request to http endpoint at /register to get a token
        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/register", address))
            .send()
            .await
            .unwrap();

        assert!(res.status().is_success());

        let token: AuthToken = res.json().await.unwrap();

        // create a websocket connection to the server
        let (mut socket, _) =
            tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap();

        let msg = socket.read_message().unwrap();
        let expected_data: Vec<u8> = ws_com_framework::Message::AuthReq {
            public_id: token.public_id,
        }
        .try_into()
        .unwrap();
        assert_eq!(msg.into_data(), expected_data);

        // send auth message with invalid passcode
        socket
            .write_message(Message::Binary(
                ws_com_framework::Message::AuthRes {
                    public_id: token.public_id,
                    passcode: "invalid".into(),
                }
                .try_into()
                .unwrap(),
            ))
            .unwrap();

        //validate we got error response
        let msg = socket.read_message().unwrap();
        let expected_data: Vec<u8> = ws_com_framework::Message::Error {
            reason: Some("failed authentication".into()),
            kind: ErrorKind::InvalidSession,
        }
        .try_into()
        .unwrap();
        assert_eq!(msg.into_data(), expected_data);

        // validate that the socket was closed
        let msg = socket.read_message().unwrap();
        assert_eq!(msg, Message::Close(None));

        //kill the std thread without waiting
        if let Err(e) = tx.send(()) {
            error!("{:?}", e);
        }
        handle.join().unwrap();

        // remove database file
        std::fs::remove_file(database_url).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[timeout(20_000)]
    async fn test_full_file_transmission() {
        init_logger();
        let database_url = String::from("./test-db-test-full-file-transmission.db");
        let port = find_open_port();
        let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
        let (_, _, handle, tx) = create_server(database_url.clone(), port).await;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // make POST request to http endpoint at /register to get a token
        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/register", address))
            .send()
            .await
            .unwrap();

        assert!(res.status().is_success());

        let token: AuthToken = res.json().await.unwrap();

        // create a websocket connection to the server
        let (mut socket, _) =
            tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap();

        let msg = socket.read_message().unwrap();
        let expected_data: Vec<u8> = ws_com_framework::Message::AuthReq {
            public_id: token.public_id,
        }
        .try_into()
        .unwrap();
        assert_eq!(msg.into_data(), expected_data);

        // send auth message to the server
        socket
            .write_message(Message::Binary(
                ws_com_framework::Message::AuthRes {
                    public_id: token.public_id,
                    passcode: token.passcode.into(),
                }
                .try_into()
                .unwrap(),
            ))
            .unwrap();

        // expect OK response
        let msg = socket.read_message().unwrap();
        let expected_bytes: Vec<u8> = ws_com_framework::Message::Ok.try_into().unwrap();
        assert_eq!(msg.into_data(), expected_bytes);

        // reuse client to send file request to server
        let get_url = format!("http://{}/agents/{}/files/{}", address, token.public_id, 24);
        let res = tokio::task::spawn(async move {
            let res = client.get(get_url).send().await.unwrap();

            info!("got response {:?}", res);
            assert!(res.status().is_success());

            res
        });

        // read messages until we get a binary message type
        let mut msg: Message = socket.read_message().unwrap();
        while !matches!(msg, Message::Binary(_)) {
            info!("got message: {:?}", msg);
            (msg, socket) = tokio::task::spawn_blocking(move || {
                let msg = socket.read_message().unwrap();
                (msg, socket)
            })
            .await
            .unwrap();
        }

        let data: ws_com_framework::Message = msg.into_data().try_into().unwrap();
        match data {
            ws_com_framework::Message::UploadTo {
                file_id,
                upload_url,
            } => {
                assert_eq!(file_id, 24);

                //upload file to server
                let upload_url = upload_url
                    .replace("localhost:8080", &address)
                    .replace("https", "http");

                info!("attempting to upload to {}", upload_url);

                let client = reqwest::Client::new();
                let res = client
                    .post(upload_url)
                    .body("hello, world")
                    .send()
                    .await
                    .unwrap();

                assert!(res.status().is_success());
            }
            m => panic!("expected file req message {:?}", m),
        }

        // validate that the file was received
        let got_res = res.await.unwrap();
        assert!(got_res.status().is_success());

        let got_res = got_res.text().await.unwrap();
        assert_eq!(got_res, "hello, world");

        //kill the std thread without waiting
        if let Err(e) = tx.send(()) {
            error!("{:?}", e);
        }
        handle.join().unwrap();

        // remove database file
        std::fs::remove_file(database_url).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[timeout(20_000)]
    async fn test_status_request() {
        init_logger();
        let database_url = String::from("./test-db-test-status-request.db");
        let port = find_open_port();
        let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
        let (_, _, handle, tx) = create_server(database_url.clone(), port).await;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // make POST request to http endpoint at /register to get a token
        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/register", address))
            .send()
            .await
            .unwrap();

        assert!(res.status().is_success());

        let token = res.json::<AuthToken>().await.unwrap();

        // create a websocket connection to the server
        let (mut socket, _) =
            tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap();

        let msg = socket.read_message().unwrap();
        let expected_data: Vec<u8> = ws_com_framework::Message::AuthReq {
            public_id: token.public_id,
        }
        .try_into()
        .unwrap();
        assert_eq!(msg.into_data(), expected_data);

        //send auth message to server
        socket
            .write_message(Message::Binary(
                ws_com_framework::Message::AuthRes {
                    public_id: token.public_id,
                    passcode: token.passcode.into(),
                }
                .try_into()
                .unwrap(),
            ))
            .unwrap();

        // expect OK response
        let msg = socket.read_message().unwrap();
        let expected_bytes: Vec<u8> = ws_com_framework::Message::Ok.try_into().unwrap();
        assert_eq!(msg.into_data(), expected_bytes);

        // reuse client to send status request to the server
        let get_url = format!("http://{}/agents/{}", address, token.public_id);
        let res = tokio::task::spawn(async move {
            let res = client.get(get_url).send().await.unwrap();

            info!("got response {:?}", res);
            assert!(res.status().is_success());

            res
        });

        // read messages until we get a binary message type
        let mut msg: Message = socket.read_message().unwrap();
        while !matches!(msg, Message::Binary(_)) {
            info!("got message: {:?}", msg);
            (msg, socket) = tokio::task::spawn_blocking(move || {
                let msg = socket.read_message().unwrap();
                (msg, socket)
            })
            .await
            .unwrap();
        }

        let data: ws_com_framework::Message = msg.into_data().try_into().unwrap();
        match data {
            ws_com_framework::Message::StatusReq {
                public_id,
                upload_id,
            } => {
                //send status response to server
                socket
                    .write_message(Message::Binary(
                        ws_com_framework::Message::StatusRes {
                            public_id,
                            ready: true,
                            uptime: 1230,
                            upload_id,
                            message: Some(String::from("hello, world - I'm alive!")),
                        }
                        .try_into()
                        .unwrap(),
                    ))
                    .unwrap();
            }
            m => panic!("expected status req message {:?}", m),
        }

        // validate that the status was received
        let got_res = res.await.unwrap();
        assert!(got_res.status().is_success());

        //validate that content type header is present and set to application/json
        let content_type = got_res.headers().get("content-type").unwrap();
        assert_eq!(content_type, "application/json");

        #[derive(Debug, Deserialize)]
        struct AgentStatus {
            ready: bool,
            uptime: u64,
            message: String,
        }

        let got_res = got_res.json::<AgentStatus>().await.unwrap();
        assert!(got_res.ready);
        assert_eq!(got_res.uptime, 1230);
        assert_eq!(got_res.message, "hello, world - I'm alive!");

        //kill the std thread without waiting
        if let Err(e) = tx.send(()) {
            error!("{:?}", e);
        }
        handle.join().unwrap();

        // remove database file
        std::fs::remove_file(database_url).unwrap();
    }
}
