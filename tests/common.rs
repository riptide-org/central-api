#![cfg(not(tarpaulin_include))]

use std::{sync::Once, task::Poll, thread::JoinHandle};

use actix_web::{
    error::PayloadError,
    web::{Bytes, Data},
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::UnboundedReceiver, oneshot, RwLock};

use central_api_lib::{
    config::Config,
    db::{Database, DbBackend},
    timelocked_hashmap::TimedHashMap,
    State,
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
        unauthenticated_servers: RwLock::new(TimedHashMap::new()),
        servers: Default::default(),
        requests: RwLock::new(TimedHashMap::new()),
        base_url: "https://localhost:8080".into(),
        start_time: std::time::Instant::now(),
    });

    let db = Data::new(
        Database::new(database_url.clone())
            .await
            .expect("a valid database connection"),
    );

    // let server_db = db.clone();
    // let server_state = state.clone();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
    let server_state = state.clone();
    let server_db = db.clone();
    let handle = std::thread::spawn(move || {
        let rt = actix_web::rt::System::new();

        let config: Config = Config {
            listener: port,
            db_url: database_url,
            base_url: "https://localhost:8080".into(),
            auth_timeout_seconds: 3,
            request_timeout_seconds: 3,
            ping_interval: 3,
            password: None,
        };

        let mut server = Box::pin(central_api_lib::start(config, server_state));

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

            server_db.close().await.unwrap();
        });
    });

    (db, state, handle, tx)
}
