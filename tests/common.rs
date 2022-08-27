#![cfg(not(tarpaulin_include))]

use std::{collections::HashMap, sync::Once, task::Poll, thread::JoinHandle};

use actix_web::{
    error::PayloadError,
    middleware::Logger,
    web::{Bytes, Data},
    App, HttpServer,
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::UnboundedReceiver, oneshot, RwLock};

use central_api::{
    db::{Database, DbBackend},
    endpoints, State,
};

static INIT: Once = Once::new();

pub fn init_logger() {
    INIT.call_once(|| {
        pretty_env_logger::init();
    });
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AuthToken {
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

pub fn find_open_port() -> std::net::TcpListener {
    for port in 1025..65535 {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            return l;
        }
    }
    panic!("no open ports found");
}

pub async fn create_server(
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
                .service(endpoints::auth::register)
                .service(endpoints::upload::upload)
                .service(endpoints::websockets::websocket)
                .configure(endpoints::info::configure)
                .configure(endpoints::download::configure)
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
