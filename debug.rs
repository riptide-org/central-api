#![feature(prelude_import)]
#[prelude_import]
use std::prelude::rust_2021::*;
#[macro_use]
extern crate std;
use std::{
    time::Duration,
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};
use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{
    HttpServer, App,
    web::{self, Bytes},
    HttpRequest, HttpResponse, get, post,
    error::PayloadError,
};
use actix_web_actors::ws;
use diesel::{
    r2d2::{ConnectionManager, self},
    SqliteConnection,
};
use futures::StreamExt;
use log::{trace, warn, info, error};
use tokio::sync::{mpsc, RwLock};
type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;
struct State {
    servers: RwLock<HashMap<String, mpsc::Sender<WsAction>>>,
    requests: RwLock<HashMap<String, mpsc::Sender<Result<Bytes, PayloadError>>>>,
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
impl WsHandler {
    fn listen_for_response(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(Duration::from_millis(50), |act, c| {
            match act.rcv.try_recv() {
                Ok(WsAction::UploadTo(msg, id)) => c.write_raw(ws::Message::Text(
                    {
                        let res = ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                            &["url ", "\nfile "],
                            &[
                                ::core::fmt::ArgumentV1::new_display(&msg),
                                ::core::fmt::ArgumentV1::new_display(&id),
                            ],
                        ));
                        res
                    }
                    .into(),
                )),
                Ok(WsAction::ServerId(id)) => c.write_raw(ws::Message::Text(
                    {
                        let res = ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                            &["your server id is "],
                            &[::core::fmt::ArgumentV1::new_display(&id)],
                        ));
                        res
                    }
                    .into(),
                )),
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
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::core::fmt::Arguments::new_v1(
                                &["received request from user and forward "],
                                &[::core::fmt::ArgumentV1::new_debug(&msg)],
                            ),
                            lvl,
                            &("actix_learning", "actix_learning", "src/main.rs", 54u32),
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
                            &("actix_learning", "actix_learning", "src/main.rs", 59u32),
                        );
                    }
                };
                self.finished(ctx);
            }
        }
    }
}
enum WsAction {
    UploadTo(String, String),
    ServerId(usize),
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::fmt::Debug for WsAction {
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        match (&*self,) {
            (&WsAction::UploadTo(ref __self_0, ref __self_1),) => {
                let debug_trait_builder = &mut ::core::fmt::Formatter::debug_tuple(f, "UploadTo");
                let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_0));
                let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_1));
                ::core::fmt::DebugTuple::finish(debug_trait_builder)
            }
            (&WsAction::ServerId(ref __self_0),) => {
                let debug_trait_builder = &mut ::core::fmt::Formatter::debug_tuple(f, "ServerId");
                let _ = ::core::fmt::DebugTuple::field(debug_trait_builder, &&(*__self_0));
                ::core::fmt::DebugTuple::finish(debug_trait_builder)
            }
        }
    }
}
#[allow(non_camel_case_types, missing_docs)]
pub struct websocket;
impl ::actix_web::dev::HttpServiceFactory for websocket {
    fn register(self, __config: &mut actix_web::dev::AppService) {
        async fn websocket(
            req: HttpRequest,
            stream: web::Payload,
            state: web::Data<State>,
        ) -> Result<HttpResponse, actix_web::Error> {
            let (tx, rx) = mpsc::channel(10);
            let server_id = state.counter.fetch_add(1, Ordering::Relaxed);
            tx.send(WsAction::ServerId(server_id)).await.unwrap();
            let mut servers = state.servers.write().await;
            servers.insert(
                {
                    let res = ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                        &[""],
                        &[::core::fmt::ArgumentV1::new_display(&server_id)],
                    ));
                    res
                },
                tx,
            );
            ws::start(WsHandler { rcv: rx }, &req, stream)
        }
        let __resource = ::actix_web::Resource::new("/websocket")
            .name("websocket")
            .guard(::actix_web::guard::Get())
            .to(websocket);
        ::actix_web::dev::HttpServiceFactory::register(__resource, __config)
    }
}
#[allow(non_camel_case_types, missing_docs)]
pub struct download;
impl ::actix_web::dev::HttpServiceFactory for download {
    fn register(self, __config: &mut actix_web::dev::AppService) {
        async fn download(req: HttpRequest, state: web::Data<State>) -> HttpResponse {
            let server_id: &str = req.match_info().get("server_id").unwrap();
            let file_id: &str = req.match_info().get("file_id").unwrap();
            let reader = state.servers.read().await;
            let server_online = reader.contains_key(server_id);
            if server_online {
                let download_id = state.counter.fetch_add(1, Ordering::Relaxed);
                let (tx, mut rx) = mpsc::channel(100);
                state.requests.write().await.insert(
                    {
                        let res = ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                            &[""],
                            &[::core::fmt::ArgumentV1::new_display(&download_id)],
                        ));
                        res
                    },
                    tx,
                );
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
                let uploader_ws = connected_servers.get(server_id).unwrap();
                uploader_ws
                    .send(WsAction::UploadTo(msg, file_id.to_string()))
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
                                    ::core::fmt::ArgumentV1::new_display(&server_id),
                                ],
                            ),
                            lvl,
                            &("actix_learning", "actix_learning", "src/main.rs", 124u32),
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
#[allow(non_camel_case_types, missing_docs)]
pub struct upload;
impl ::actix_web::dev::HttpServiceFactory for upload {
    fn register(self, __config: &mut actix_web::dev::AppService) {
        async fn upload(
            req: HttpRequest,
            mut payload: web::Payload,
            state: web::Data<State>,
        ) -> HttpResponse {
            let upload_id: &str = req.match_info().get("upload_id").unwrap();
            let mut sender_store = state.requests.write().await;
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["the upload id is: ", "\n"],
                    &[::core::fmt::ArgumentV1::new_display(&upload_id)],
                ));
            };
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["available keys are: ", "\n"],
                    &[::core::fmt::ArgumentV1::new_debug(&sender_store.keys())],
                ));
            };
            let sender = sender_store.remove(upload_id).unwrap();
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
                                &("actix_learning", "actix_learning", "src/main.rs", 144u32),
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
fn main() -> std::io::Result<()> {
    <::actix_web::rt::System>::new().block_on(async move {
        {
            let state = web::Data::new(State {
                servers: Default::default(),
                requests: RwLock::new(HashMap::new()),
                counter: AtomicUsize::new(0),
                base_url: "http://127.0.0.1:8080".into(),
            });
            HttpServer::new(move || {
                App::new()
                    .app_data(state.clone())
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
