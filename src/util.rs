//! Various utility functions used throughout the application

use actix_web::web::Data;
use tokio::task::JoinHandle;

use crate::{endpoints::websockets::InternalComm, State};

/// Start a watcher to automatically remove expired auth and request tokens from the state of the application
pub fn start_watcher(
    state: Data<State>,
    auth_timeout: u64,
) -> JoinHandle<Result<(), Box<dyn std::error::Error>>> {
    actix_web::rt::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let mut unauthenticated_servers = state.unauthenticated_servers.write().await;

            // while the last element in the hashmap is expired, remove it
            while unauthenticated_servers
                .front_with_time()
                .map(|(_, _, i)| i.elapsed().as_secs() > auth_timeout)
                .unwrap_or(false)
            {
                if let Some(server) = unauthenticated_servers.pop_front() {
                    server.1.send(InternalComm::CloseConnection).await.unwrap();
                }
            }
        }
    })
}
