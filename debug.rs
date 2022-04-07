#![feature(prelude_import)]
#[prelude_import]
use std::prelude::rust_2021::*;
#[macro_use]
extern crate std;
use std::collections::HashMap;
use actix_web::{
    HttpServer, App,
    web::{self, Bytes},
    HttpRequest, HttpResponse, get, post,
    error::PayloadError,
};
use db::MockDB;
use diesel::{
    r2d2::{ConnectionManager, self},
    SqliteConnection,
};
use futures::StreamExt;
use log::{trace, warn, info, error};
use rand::Rng;
use tokio::sync::{mpsc, RwLock};
use websockets::{WsAction, websocket};
use ws_com_framework::{FileId, PublicId as ServerId, Passcode, Message};
mod db {
    use crate::ServerId;
    pub struct MockDB {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::fmt::Debug for MockDB {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match *self {
                MockDB {} => {
                    let debug_trait_builder =
                        &mut ::core::fmt::Formatter::debug_struct(f, "MockDB");
                    ::core::fmt::DebugStruct::finish(debug_trait_builder)
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::clone::Clone for MockDB {
        #[inline]
        fn clone(&self) -> MockDB {
            {
                *self
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::marker::Copy for MockDB {}
    impl DbBackend for MockDB {
        fn save_entry(server_id: ServerId, passcode: [u8; 32]) -> Result<(), DbBackendError>
        where
            Self: Sized,
        {
            ::core::panicking::panic("not yet implemented")
        }
        fn validate_server(server_id: ServerId, passcode: &[u8; 32]) -> Result<bool, DbBackendError>
        where
            Self: Sized,
        {
            ::core::panicking::panic("not yet implemented")
        }
        fn contains_entry(server_id: ServerId) -> Result<bool, DbBackendError>
        where
            Self: Sized,
        {
            ::core::panicking::panic("not yet implemented")
        }
    }
    pub trait DbBackend: Clone {
        fn save_entry(server_id: ServerId, passcode: [u8; 32]) -> Result<(), DbBackendError>
        where
            Self: Sized;
        fn validate_server(
            server_id: ServerId,
            passcode: &[u8; 32],
        ) -> Result<bool, DbBackendError>
        where
            Self: Sized;
        fn contains_entry(server_id: ServerId) -> Result<bool, DbBackendError>
        where
            Self: Sized;
    }
    pub enum DbBackendError {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::fmt::Debug for DbBackendError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            unsafe { ::core::intrinsics::unreachable() }
        }
    }
}
mod websockets {
    use std::time::Duration;
    use actix::{Actor, AsyncContext, StreamHandler};
    use actix_web::{get, web, HttpRequest, HttpResponse};
    use actix_web_actors::ws::{self, CloseReason};
    use log::{info, warn, error};
    use tokio::sync::mpsc;
    use ws_com_framework::Message;
    use crate::{
        FileId, ServerId, State,
        db::{self, DbBackend},
    };
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
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::fmt::Debug for WsAction {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match (&*self,) {
                (&WsAction::UploadTo(ref __self_0, ref __self_1),) => {
                    let debug_trait_builder =
                        &mut ::core::fmt::Formatter::debug_tuple(f, "UploadTo");
                    let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_0));
                    let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_1));
                    ::core::fmt::DebugTuple::finish(debug_trait_builder)
                }
                (&WsAction::UploadMetadata(ref __self_0, ref __self_1),) => {
                    let debug_trait_builder =
                        &mut ::core::fmt::Formatter::debug_tuple(f, "UploadMetadata");
                    let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_0));
                    let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_1));
                    ::core::fmt::DebugTuple::finish(debug_trait_builder)
                }
                (&WsAction::CloseConnection(ref __self_0),) => {
                    let debug_trait_builder =
                        &mut ::core::fmt::Formatter::debug_tuple(f, "CloseConnection");
                    let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_0));
                    ::core::fmt::DebugTuple::finish(debug_trait_builder)
                }
                (&WsAction::RequestAuthentication(ref __self_0),) => {
                    let debug_trait_builder =
                        &mut ::core::fmt::Formatter::debug_tuple(f, "RequestAuthentication");
                    let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_0));
                    ::core::fmt::DebugTuple::finish(debug_trait_builder)
                }
                (&WsAction::RequestStatus,) => {
                    ::core::fmt::Formatter::write_str(f, "RequestStatus")
                }
            }
        }
    }
    pub struct WsHandler {
        rcv: mpsc::Receiver<Message>,
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::fmt::Debug for WsHandler {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match *self {
                WsHandler {
                    rcv: ref __self_0_0,
                } => {
                    let debug_trait_builder =
                        &mut ::core::fmt::Formatter::debug_struct(f, "WsHandler");
                    let _ = ::core::fmt::DebugStruct::field(
                        debug_trait_builder,
                        "rcv",
                        &&(*__self_0_0),
                    );
                    ::core::fmt::DebugStruct::finish(debug_trait_builder)
                }
            }
        }
    }
    impl Actor for WsHandler {
        type Context = ws::WebsocketContext<Self>;
        fn started(&mut self, ctx: &mut Self::Context) {
            self.listen_for_response(ctx);
        }
    }
    impl WsHandler {
        fn listen_for_response(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
            ctx.run_interval(Duration::from_millis(50), |act, c| {
                match act.rcv.try_recv() {
                    Ok(Message::Close) => {
                        c.close(None);
                        act.finished(c);
                    }
                    Ok(Message::Error(reason, end_connection, kind)) => {
                        let msg = Message::Error(reason, end_connection, kind);
                        {
                            if let Ok(data) = TryInto::<Vec<u8>>::try_into(msg) {
                                c.write_raw(ws::Message::Binary(actix_web::web::Bytes::from(data)));
                            } else {
                                {
                                    let lvl = ::log::Level::Error;
                                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                                        ::log::__private_api_log(
                                            ::core::fmt::Arguments::new_v1(
                                                &["failed to send message down websocket "],
                                                &[::core::fmt::ArgumentV1::new_debug(&msg)],
                                            ),
                                            lvl,
                                            &(
                                                "actix_learning::websockets",
                                                "actix_learning::websockets",
                                                "src/websockets.rs",
                                                69u32,
                                            ),
                                        );
                                    }
                                };
                            }
                        };
                        if Into::<bool>::into(end_connection) {
                            {
                                c.close(None);
                                act.finished(c);
                            }
                        }
                    }
                    Ok(msg) => {
                        if let Ok(data) = TryInto::<Vec<u8>>::try_into(msg) {
                            c.write_raw(ws::Message::Binary(actix_web::web::Bytes::from(data)));
                        } else {
                            {
                                let lvl = ::log::Level::Error;
                                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                                    ::log::__private_api_log(
                                        ::core::fmt::Arguments::new_v1(
                                            &["failed to send message down websocket "],
                                            &[::core::fmt::ArgumentV1::new_debug(&msg)],
                                        ),
                                        lvl,
                                        &(
                                            "actix_learning::websockets",
                                            "actix_learning::websockets",
                                            "src/websockets.rs",
                                            74u32,
                                        ),
                                    );
                                }
                            };
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => act.finished(c),
                }
            });
        }
    }
    impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsHandler {
        fn handle(
            &mut self,
            item: Result<ws::Message, ws::ProtocolError>,
            ctx: &mut Self::Context,
        ) {
            match item {
                Ok(msg) => {
                    {
                        let lvl = ::log::Level::Info;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api_log(
                                ::core::fmt::Arguments::new_v1(
                                    &["received request from user "],
                                    &[::core::fmt::ArgumentV1::new_debug(&msg)],
                                ),
                                lvl,
                                &(
                                    "actix_learning::websockets",
                                    "actix_learning::websockets",
                                    "src/websockets.rs",
                                    86u32,
                                ),
                            );
                        }
                    };
                }
                Err(e) => {
                    {
                        let lvl = ::log::Level::Warn;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api_log(
                                ::core::fmt::Arguments::new_v1(
                                    &[
                                        "received an error from client ",
                                        "\ngoing to stop connection",
                                    ],
                                    &[::core::fmt::ArgumentV1::new_debug(&e)],
                                ),
                                lvl,
                                &(
                                    "actix_learning::websockets",
                                    "actix_learning::websockets",
                                    "src/websockets.rs",
                                    89u32,
                                ),
                            );
                        }
                    };
                    self.finished(ctx);
                }
            }
        }
    }
    #[allow(non_camel_case_types, missing_docs)]
    pub struct websocket;
    impl ::actix_web::dev::HttpServiceFactory for websocket {
        fn register(self, __config: &mut actix_web::dev::AppService) {
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
                    return Ok(HttpResponse::Forbidden()
                        .body("another server is already authenticated with this id"));
                }
                let mut servers = state.unauthenticated_servers.write().await;
                servers.insert(server_id, tx);
                ws::start(WsHandler { rcv: rx }, &req, stream)
            }
            let __resource = ::actix_web::Resource::new("/ws/{server_id}")
                .name("websocket")
                .guard(::actix_web::guard::Get())
                .to(websocket);
            ::actix_web::dev::HttpServiceFactory::register(__resource, __config)
        }
    }
}
type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;
type RequestId = u64;
pub struct State {
    unauthenticated_servers: RwLock<HashMap<ServerId, mpsc::Sender<Message>>>,
    servers: RwLock<HashMap<ServerId, mpsc::Sender<Message>>>,
    requests: RwLock<HashMap<RequestId, mpsc::Sender<Result<Bytes, PayloadError>>>>,
    base_url: String,
}
/// Download a file from a client
#[allow(non_camel_case_types, missing_docs)]
pub struct download;
impl ::actix_web::dev::HttpServiceFactory for download {
    fn register(self, __config: &mut actix_web::dev::AppService) {
        /// Download a file from a client
        async fn download(
            req: HttpRequest,
            state: web::Data<State>,
            path: web::Path<(ServerId, FileId)>,
        ) -> HttpResponse {
            let (server_id, file_id) = path.into_inner();
            let reader = state.servers.read().await;
            let server_online = reader.contains_key(&server_id);
            if server_online {
                let (tx, mut rx) = mpsc::channel(100);
                let download_id = rand::thread_rng().gen();
                state.requests.write().await.insert(download_id, tx);
                let msg = {
                    let res = ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                        &["", "/upload/"],
                        &[
                            ::core::fmt::ArgumentV1::new_display(&state.base_url),
                            ::core::fmt::ArgumentV1::new_display(&download_id),
                        ],
                    ));
                    res
                };
                let connected_servers = state.servers.read().await;
                let uploader_ws = connected_servers.get(&server_id).unwrap();
                uploader_ws
                    .send(Message::UploadTo(file_id, msg))
                    .await
                    .unwrap();
                let payload = {
                    let (mut __yield_tx, __yield_rx) = ::async_stream::yielder::pair();
                    ::async_stream::AsyncStream::new(__yield_rx, async move {
                        while let Some(v) = rx.recv().await {
                            __yield_tx.send(v).await;
                        }
                    })
                };
                HttpResponse::Ok()
                    .content_type("text/html")
                    .streaming(payload)
            } else {
                {
                    let lvl = ::log::Level::Trace;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::core::fmt::Arguments::new_v1(
                                &[
                                    "client attempted to request file ",
                                    " from ",
                                    ", but that server isn\'t connected",
                                ],
                                &[
                                    ::core::fmt::ArgumentV1::new_display(&file_id),
                                    ::core::fmt::ArgumentV1::new_debug(&server_id),
                                ],
                            ),
                            lvl,
                            &("actix_learning", "actix_learning", "src/main.rs", 64u32),
                        );
                    }
                };
                HttpResponse::NotFound()
                    .content_type("text/html")
                    .body("requested resource not found, the server may not be connected")
            }
        }
        let __resource = ::actix_web::Resource::new("/download/{server_id}/{file_id}")
            .name("download")
            .guard(::actix_web::guard::Get())
            .to(download);
        ::actix_web::dev::HttpServiceFactory::register(__resource, __config)
    }
}
/// Upload a file or metadata to a waiting client
#[allow(non_camel_case_types, missing_docs)]
pub struct upload;
impl ::actix_web::dev::HttpServiceFactory for upload {
    fn register(self, __config: &mut actix_web::dev::AppService) {
        /// Upload a file or metadata to a waiting client
        async fn upload(
            req: HttpRequest,
            mut payload: web::Payload,
            state: web::Data<State>,
            path: web::Path<ServerId>,
        ) -> HttpResponse {
            let upload_id = path.into_inner();
            let mut sender_store = state.requests.write().await;
            ::std::io::_print(::core::fmt::Arguments::new_v1(
                &["the upload id is: ", "\n"],
                &[::core::fmt::ArgumentV1::new_debug(&upload_id)],
            ));
            ::std::io::_print(::core::fmt::Arguments::new_v1(
                &["available keys are: ", "\n"],
                &[::core::fmt::ArgumentV1::new_debug(&sender_store.keys())],
            ));
            let sender = sender_store.remove(&upload_id).unwrap();
            while let Some(chk) = payload.next().await {
                if let Err(e) = sender.send(chk).await {
                    {
                        let lvl = ::log::Level::Error;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api_log(
                                ::core::fmt::Arguments::new_v1(
                                    &["problem sending payload "],
                                    &[::core::fmt::ArgumentV1::new_debug(&e)],
                                ),
                                lvl,
                                &("actix_learning", "actix_learning", "src/main.rs", 85u32),
                            );
                        }
                    };
                    return HttpResponse::InternalServerError().body("upload failed");
                };
            }
            HttpResponse::Ok().body("succesfully uploaded")
        }
        let __resource = ::actix_web::Resource::new("/upload/{upload_id}")
            .name("upload")
            .guard(::actix_web::guard::Post())
            .to(upload);
        ::actix_web::dev::HttpServiceFactory::register(__resource, __config)
    }
}
/// Attempt to register a new webserver with the api
#[allow(non_camel_case_types, missing_docs)]
pub struct register;
impl ::actix_web::dev::HttpServiceFactory for register {
    fn register(self, __config: &mut actix_web::dev::AppService) {
        /// Attempt to register a new webserver with the api
        pub async fn register() -> HttpResponse {
            ::core::panicking::panic("not yet implemented")
        }
        let __resource = ::actix_web::Resource::new("/register")
            .name("register")
            .guard(::actix_web::guard::Post())
            .to(register);
        ::actix_web::dev::HttpServiceFactory::register(__resource, __config)
    }
}
fn main() -> std::io::Result<()> {
    <::actix_web::rt::System>::new().block_on(async move {
        {
            pretty_env_logger::init();
            let state = web::Data::new(State {
                unauthenticated_servers: Default::default(),
                servers: Default::default(),
                requests: RwLock::new(HashMap::new()),
                base_url: "http://127.0.0.1:8080".into(),
            });
            let database = MockDB {};
            HttpServer::new(move || {
                App::new()
                    .app_data(state.clone())
                    .app_data(database.clone())
                    .service(websocket)
                    .service(download)
                    .service(upload)
            })
            .bind(("127.0.0.1", 8080))?
            .run()
            .await
        }
    })
}
