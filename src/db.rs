use crate::error::Error;
use crate::structs::{Agent, AgentRequest, AgentUpdateRequest};
use mobc::{Connection, Pool};
use mobc_postgres::{tokio_postgres, PgConnectionManager};
use std::fs;
use std::str::FromStr;
use std::time::Duration;
use tokio_postgres::{Config, NoTls, Row};

const DB_POOL_MAX_OPEN: u64 = 32;
const DB_POOL_MAX_IDLE: u64 = 8;
const DB_POOL_TIMEOUT_SECONDS: u64 = 15;
const INIT_SQL: &str = "./config/db.sql";

pub type DBCon = Connection<PgConnectionManager<NoTls>>;
pub type DBPool = Pool<PgConnectionManager<NoTls>>;

pub trait FromDataBase: Sized {
    type Error: Send + std::fmt::Debug + Into<Error>;
    fn from_database(data: &Row) -> Result<Self, Self::Error>;
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
    let conn = get_db_con(pool).await?;
    conn.batch_execute(&init_file)
        .await
        .map_err(Error::DBInitError)?;
    Ok(())
}

pub enum Search {
    Id(usize),
    unique_id(String),
}

impl Search {
    fn get_search_term(self) -> String {
        match self {
            Search::Id(i) => format!("{} = {}", "id", i),
            Search::unique_id(s) => format!("{} = '{}'", "unique_id", s),
        }
    }

    pub async fn find(self, db_pool: &DBPool) -> Result<Option<Agent>, Error> {
        let mut s = search_database(db_pool, self).await?;
        if s.is_empty() {
            return Ok(None);
        }
        Ok(Some(s.remove(0)))
    }
}

async fn search_database(db_pool: &DBPool, search: Search) -> Result<Vec<Agent>, Error> {
    let conn = get_db_con(db_pool).await?;

    let rows = conn
        .query(
            format!(
                "
        SELECT * from agents
        WHERE {}
        ORDER BY created_at DESC
    ",
                search.get_search_term()
            )
            .as_str(),
            &[],
        )
        .await
        .map_err(Error::DBQueryError)?;

    rows.iter().map(|r| Agent::from_database(r)).collect()
}

pub async fn add_agent(db_pool: &DBPool, body: AgentRequest) -> Result<Agent, Error> {
    let conn = get_db_con(db_pool).await?;
    let row = conn
        .query_one(
            "
        INSERT INTO agents (unique_id)
        VALUES ($1)
        RETURNING *;
    ",
            &[&body.unique_id()],
        )
        .await
        .map_err(Error::DBQueryError)?;

    Agent::from_database(&row)
}

pub async fn update_agent(
    db_pool: &DBPool,
    id: &usize,
    body: AgentUpdateRequest,
) -> Result<Agent, Error> {
    let conn = get_db_con(db_pool).await?;
    let row = conn
        .query_one(
            "
            UPDATE agents
            SET last_signin = $1
            WHERE id = $2
            RETURNING *;
        ",
            &[&body.last_signin(), &(*id as i64)],
        )
        .await
        .map_err(Error::DBQueryError)?;

    Agent::from_database(&row)
}

#[allow(dead_code)]
pub async fn delete_agent(db_pool: &DBPool, id: &usize) -> Result<u64, Error> {
    let conn = get_db_con(db_pool).await?;
    conn.execute(
        "
            DELETE FROM agents
            WHERE id = $1
        ",
        &[&(*id as i64)],
    )
    .await
    .map_err(Error::DBQueryError)
}
