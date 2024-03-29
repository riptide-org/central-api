//! The database backing the central-api

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::UNIX_EPOCH,
};

use crate::{
    models::{Agent, *},
    ServerId,
};
use actix_web::web::{self, Data};
use async_trait::async_trait;
use diesel::{
    r2d2::{ConnectionManager, Pool},
    ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use log::trace;
use sha2::{Digest, Sha256};
use ws_com_framework::Passcode;

/// Stores the sql migrations to setup the database on first launch
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

/// Shorthand for the database connection pool
type DbPool = Pool<ConnectionManager<SqliteConnection>>;

/// A trait which allows for the database to be used in a generic way
#[async_trait]
pub trait DbBackend {
    /// `new` will attempt
    async fn new<S: Into<String> + Send>(database_url: S) -> Result<Self, DbBackendError>
    where
        Self: Sized;

    /// `save_entry` will take a passcode and `ServerId` and save it into the server.
    async fn save_entry(
        &self,
        server_id: ServerId,
        passcode: Passcode,
    ) -> Result<Option<Passcode>, DbBackendError>;

    /// `validate_server` will take a provided `ServerId` and return true if the provided passcode matches.
    /// It will return false if the provided passcode fails validation.
    async fn validate_server(
        &self,
        server_id: &ServerId,
        passcode: &Passcode,
    ) -> Result<bool, DbBackendError>;

    /// `contains_entry` will validate whether the provided `ServerId` exists in the database
    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError>;

    /// `update_last_seen` will update the last seen time for the provided `ServerId`
    async fn update_last_seen(&self, server_id: &ServerId) -> Result<(), DbBackendError>;

    /// `close` will attempt to destroy this connection to the database
    async fn close(self) -> Result<(), DbBackendError>;
}

/// The database handler for the central api, wraps over a connection `DbPool` to allow asynchronous actions
#[derive(Debug)]
pub struct Database(DbPool);

impl Database {
    /// Attempt to initalise the database with generic sql
    pub async fn init(&self) -> Result<(), DbBackendError> {
        let mut conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;

        web::block(move || {
            conn.exclusive_transaction(|conn| conn.run_pending_migrations(MIGRATIONS).map(|_| ()))
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        Ok(())
    }
}

impl Deref for Database {
    type Target = DbPool;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Database {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[async_trait]
impl DbBackend for Database {
    async fn new<S: Into<String> + Send>(database_url: S) -> Result<Self, DbBackendError> {
        let db = Self(
            Pool::builder()
                .build(ConnectionManager::<SqliteConnection>::new(database_url))
                .map_err(|e| DbBackendError::InitFailed(e.to_string()))?,
        );
        db.init().await?;
        Ok(db)
    }

    async fn save_entry(
        &self,
        server_id: ServerId,
        passcode: Passcode,
    ) -> Result<Option<Passcode>, DbBackendError> {
        use crate::schema::agents::dsl::*;

        let mut hasher = Sha256::new();
        hasher.update(passcode);

        let new_agent: NewAgent = NewAgent {
            public_id: server_id as i64,
            secure_key: hasher.finalize().to_vec(), //hash passcode
            last_seen: std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_secs() as i64,
        };

        let mut conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;

        //run in a blocking context
        let existing_agent: Result<Option<Agent>, diesel::result::Error> = web::block(move || {
            //attempt to get the existing agent if it exists
            let ex: Option<Agent> = agents
                .filter(public_id.eq(new_agent.public_id))
                .first::<Agent>(&mut conn)
                .optional()?;

            // upsert the agent, replacing passcode if it exists
            diesel::insert_into(agents)
                .values(&new_agent)
                .on_conflict(public_id)
                .do_update()
                .set(secure_key.eq(&new_agent.secure_key))
                .execute(&mut conn)?;

            Ok(ex)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        Ok(existing_agent
            .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
            .map(|a| a.secure_key))
    }

    async fn validate_server(
        &self,
        server_id: &ServerId,
        passcode: &Passcode,
    ) -> Result<bool, DbBackendError> {
        use crate::schema::agents::dsl::*;

        let mut conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;
        let server_id: i64 = *server_id as i64;
        let users = web::block(move || {
            agents
                .filter(public_id.eq(server_id))
                .load::<Agent>(&mut conn)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        if !users.is_empty() {
            // UNIQUE constraint means there will never be more than 1 result
            let cmp = &users[0].secure_key;
            let mut hasher = Sha256::new();
            hasher.update(passcode);
            Ok(hasher.finalize().to_vec() == *cmp)
        } else {
            Ok(false)
        }
    }

    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
        use crate::schema::agents::dsl::*;

        let mut conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;
        let server_id: i64 = *server_id as i64;

        let users: i64 = web::block(move || {
            agents
                .filter(public_id.eq(server_id))
                .count()
                .get_result(&mut conn)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        Ok(users != 0)
    }

    async fn update_last_seen(&self, server_id: &ServerId) -> Result<(), DbBackendError> {
        use crate::schema::agents::dsl::*;

        let mut conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;
        let server_id: i64 = *server_id as i64;

        web::block(move || {
            diesel::update(agents.filter(public_id.eq(server_id)))
                .set(
                    last_seen.eq(std::time::SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("time went backwards")
                        .as_secs() as i64),
                )
                .execute(&mut conn)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn close(self) -> Result<(), DbBackendError> {
        Ok(())
    }
}

#[async_trait]
impl<T: DbBackend + Send + Sync> DbBackend for Data<T> {
    async fn new<S: Into<String> + Send>(database_url: S) -> Result<Data<T>, DbBackendError> {
        Ok(Data::new(T::new(database_url).await?))
    }

    async fn save_entry(
        &self,
        server_id: ServerId,
        passcode: Passcode,
    ) -> Result<Option<Passcode>, DbBackendError> {
        self.deref().save_entry(server_id, passcode).await
    }

    async fn validate_server(
        &self,
        server_id: &ServerId,
        passcode: &Passcode,
    ) -> Result<bool, DbBackendError> {
        self.deref().validate_server(server_id, passcode).await
    }

    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
        self.deref().contains_entry(server_id).await
    }

    async fn update_last_seen(&self, server_id: &ServerId) -> Result<(), DbBackendError> {
        self.deref().update_last_seen(server_id).await
    }

    async fn close(self) -> Result<(), DbBackendError> {
        match std::sync::Arc::<T>::try_unwrap(self.into_inner()) {
            Ok(r) => r.close().await,
            Err(_) => {
                trace!("attempted closure with more than 1 strong reference");
                Ok(())
            }
        }
    }
}

#[async_trait]
impl<T: DbBackend + Send + Sync> DbBackend for Arc<T> {
    async fn new<S: Into<String> + Send>(database_url: S) -> Result<Arc<T>, DbBackendError> {
        Ok(Arc::new(T::new(database_url).await?))
    }

    async fn save_entry(
        &self,
        server_id: ServerId,
        passcode: Passcode,
    ) -> Result<Option<Passcode>, DbBackendError> {
        self.deref().save_entry(server_id, passcode).await
    }

    async fn validate_server(
        &self,
        server_id: &ServerId,
        passcode: &Passcode,
    ) -> Result<bool, DbBackendError> {
        self.deref().validate_server(server_id, passcode).await
    }

    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
        self.deref().contains_entry(server_id).await
    }

    async fn update_last_seen(&self, server_id: &ServerId) -> Result<(), DbBackendError> {
        self.deref().update_last_seen(server_id).await
    }

    async fn close(self) -> Result<(), DbBackendError> {
        match std::sync::Arc::<T>::try_unwrap(self) {
            Ok(r) => r.close().await,
            Err(_) => {
                trace!("attempted closure with more than 1 strong reference");
                Ok(())
            }
        }
    }
}

/// Represents an error generated by the database for the central api.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbBackendError {
    /// The item that was attempted to insert already exists.
    AlreadyExist,
    /// There is no connection available to the database.
    NoConnectionAvailable(String),
    /// The query failed.
    QueryFailed(String),
    /// Initalisation of the database failed
    InitFailed(String),
}

impl std::fmt::Display for DbBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbBackendError::AlreadyExist => write!(f, "entry already exists"),
            DbBackendError::NoConnectionAvailable(e) => {
                write!(f, "connection is unavailable due to error: {}", e)
            }
            DbBackendError::QueryFailed(e) => write!(f, "query failed due to error: {}", e),
            DbBackendError::InitFailed(e) => {
                write!(f, "initalise database failed due to error: {}", e)
            }
        }
    }
}

impl std::error::Error for DbBackendError {}

#[cfg(test)]
#[cfg(not(tarpaulin_include))]
pub mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use tokio::sync::RwLock;

    use super::{DbBackend, DbBackendError};
    use crate::ServerId;
    use ws_com_framework::Passcode;

    /// An entirely insecure mock implementation of a backend database for development and testing purposes
    #[derive(Debug)]
    pub struct MockDb {
        store: RwLock<HashMap<ServerId, Passcode>>,
    }

    #[async_trait]
    impl DbBackend for MockDb {
        async fn new<S: Into<String> + Send>(_: S) -> Result<Self, DbBackendError> {
            Ok(Self {
                store: Default::default(),
            })
        }

        async fn save_entry(
            &self,
            server_id: ServerId,
            passcode: Passcode,
        ) -> Result<Option<Passcode>, DbBackendError> {
            Ok(self.store.write().await.insert(server_id, passcode))
        }

        async fn validate_server(
            &self,
            server_id: &ServerId,
            passcode: &Passcode,
        ) -> Result<bool, DbBackendError> {
            match self.store.read().await.get(server_id) {
                Some(s) if s == passcode => Ok(true),
                _ => Ok(false),
            }
        }

        async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
            match self.store.read().await.get(server_id) {
                Some(_) => Ok(true),
                None => Ok(false),
            }
        }

        async fn update_last_seen(&self, _: &ServerId) -> Result<(), DbBackendError> {
            Ok(())
        }

        async fn close(self) -> Result<(), DbBackendError> {
            Ok(())
        }
    }
}
