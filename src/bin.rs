use actix_web::web::Data;
use central_api_lib::{config::Config, timelocked_hashmap::TimedHashMap, *};
use tokio::sync::RwLock;

#[doc(hidden)]
#[actix_web::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();
    let config = Config::load()?;
    let state = Data::new(State {
        unauthenticated_servers: RwLock::new(TimedHashMap::new()),
        servers: Default::default(),
        requests: RwLock::new(TimedHashMap::new()),
        base_url: config.base_url.clone(),
        start_time: std::time::Instant::now(),
    });
    start(config, state).await?;
    Ok(())
}
