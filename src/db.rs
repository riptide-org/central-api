use mobc_postgres::{tokio_postgres, PgConnectionManager};
use tokio_postgres::{Config, NoTls, Row};
use std::str::FromStr;
use std::time::Duration;
use mobc::{Connection, Pool};
use std::fs;
use crate::error::Error;
use crate::structs::{Agent, AgentRequest, AgentResponse, AgentUpdateRequest};

const DB_POOL_MAX_OPEN: u64 = 32;
const DB_POOL_MAX_IDLE: u64 = 8;
const DB_POOL_TIMEOUT_SECONDS: u64 = 15;
const INIT_SQL: &str = "./config/db.sql";

pub type DBCon = Connection<PgConnectionManager<NoTls>>;
pub type DBPool = Pool<PgConnectionManager<NoTls>>;

pub trait FromDataBase: Sized {
    type Error: Send + std::fmt::Debug + Into<Error>;
    fn from_database(data: Row) -> Result<Self, Self::Error>;
}

pub fn create_pool() -> Result<DBPool, mobc::Error<tokio_postgres::Error>> {
    let config = Config::from_str("postgres://postgres@127.0.0.1:7877/postgres")?; //TODO load this from config file

    let manager = PgConnectionManager::new(config, NoTls);
    Ok(Pool::builder()
            .max_open(DB_POOL_MAX_OPEN)
            .max_idle(DB_POOL_MAX_IDLE)
            .get_timeout(Some(Duration::from_secs(DB_POOL_TIMEOUT_SECONDS)))
            .build(manager))
}

pub async fn get_db_con(pool: &DBPool) -> Result<DBCon, Error> {
    pool.get().await.map_err(Error::DBPoolError)
}

pub async fn init_db(pool: &DBPool) -> Result<(), Error> {
    let init_file = fs::read_to_string(INIT_SQL)?;
    let con = get_db_con(pool).await?;
    con
        .batch_execute(&init_file)
        .await
        .map_err(Error::DBInitError)?;
    Ok(())
}

pub async fn create_agent(db_pool: &DBPool, body: AgentRequest) -> Result<impl FromDataBase, Error> {
    let con = get_db_con(db_pool).await?;
    let row = con.query_one("
        INSERT INTO agents (unique_id)
        VALUES ($1)
        RETURNING *;
    ", &[&body.unique_id()])
        .await
        .map_err(Error::DBQueryError)?;

    Agent::from_database(row)
}

pub async fn update_agent(db_pool: &DBPool, id: &i64, body: AgentUpdateRequest) -> Result<impl FromDataBase, Error> {
    let con = get_db_con(db_pool).await?;
    let row = con
        .query_one("
            UPDATE agents
            SET last_signin = $1
            WHERE id = $2
            RETURNING *;
        ", &[body.last_signin(), id])
        .await
        .map_err(Error::DBQueryError)?;

    Agent::from_database(row)
}

#[allow(dead_code)]
pub async fn delete_agent(db_pool: &DBPool, id: &i64) -> Result<u64, Error> {
    let con = get_db_con(db_pool).await?;
    con
        .execute("
            DELETE FROM agents
            WHERE id = $1
        ", &[id])
        .await
        .map_err(Error::DBQueryError)
}