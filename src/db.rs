use mobc_postgres::{tokio_postgres, PgConnectionManager};
use tokio_postgres::{Config, Error, NoTls};
use std::str::FromStr;
use std::time::Duration;
use mobc::{Connection, Pool};
use std::fs;

pub type DBCon = Connection<PgConnectionManager<NoTls>>;
pub type DBPool = Pool<PgConnectionManager<NoTls>>;

const DB_POOL_MAX_OPEN: u64 = 32;
const DB_POOL_MAX_IDLE: u64 = 8;
const DB_POOL_TIMEOUT_SECONDS: u64 = 15;
const INIT_SQL: &str = "./config/db.sql";

pub fn create_pool() -> Result<DBPool, mobc::Error<Error>> { //TODO wrap this error
    let config = Config::from_str("postgres://postgres@127.0.0.1:7878/postgres")?; //TODO load this from config file

    let manager = PgConnectionManager::new(config, NoTls);
    Ok(Pool::builder()
            .max_open(DB_POOL_MAX_OPEN)
            .max_idle(DB_POOL_MAX_IDLE)
            .get_timeout(Some(Duration::from_secs(DB_POOL_TIMEOUT_SECONDS)))
            .build(manager))
}

pub async fn get_db_con(pool: &DBPool) -> Result<DBCon, ()> { //TODO Error handling
    pool.get().await.map_err(|_| ())
}

pub async fn init_db(pool: &DBPool) -> Result<(), ()> { //TODO Error handling
    let init_file = fs::read_to_string(INIT_SQL).unwrap();
    let con = get_db_con(pool).await?;
    con
        .batch_execute(&init_file)
        .await
        .map_err(|_| ())?;
    Ok(())
}