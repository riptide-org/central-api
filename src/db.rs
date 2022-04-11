use std::{collections::HashMap, ops::Deref, sync::Arc};

use crate::{models::*, ServerId};
use actix_web::web::{self, Data};
use async_trait::async_trait;
use diesel::{
    r2d2::{ConnectionManager, Pool},
    ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection,
};
use log::trace;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use ws_com_framework::Passcode;

type DbPool = Pool<ConnectionManager<SqliteConnection>>;

pub struct Database(DbPool);

/// An entirely insecure mock implementation of a backend database for development and testing purposes
pub struct MockDb {
    store: RwLock<HashMap<ServerId, Passcode>>,
}

#[async_trait]
pub trait DbBackend {
    /// `new` will attempt
    async fn new() -> Result<Self, DbBackendError>
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

    /// `close` will attempt to destroy this connection to the database
    async fn close(self) -> Result<(), DbBackendError>;
}

#[async_trait]
impl DbBackend for Database {
    async fn new() -> Result<Self, DbBackendError> {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        Ok(Self(
            Pool::builder()
                .build(ConnectionManager::<SqliteConnection>::new(&database_url))
                .map_err(|e| DbBackendError::InitFailed(e.to_string()))?,
        ))
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
        };

        let conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;

        //TODO: check if exists already and save that passcode to return as Some(Passcode) at end

        //run in a blocking context
        web::block(move || {
            diesel::insert_into(agents)
                .values(&new_agent)
                .execute(&conn)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        Ok(None)
    }

    async fn validate_server(
        &self,
        server_id: &ServerId,
        passcode: &Passcode,
    ) -> Result<bool, DbBackendError> {
        use crate::schema::agents::dsl::*;

        let conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;
        let server_id: i64 = *server_id as i64;
        let users = web::block(move || {
            agents
                .filter(public_id.eq(server_id))
                .load::<crate::models::Agent>(&conn)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        if users.len() > 1 {
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

        let conn = self
            .0
            .get()
            .map_err(|e| DbBackendError::NoConnectionAvailable(e.to_string()))?;
        let server_id: i64 = *server_id as i64;

        let users: i64 = web::block(move || {
            agents
                .filter(public_id.eq(server_id))
                .count()
                .get_result(&conn)
        })
        .await
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?
        .map_err(|e| DbBackendError::QueryFailed(e.to_string()))?;

        Ok(users != 0)
    }

    async fn close(self) -> Result<(), DbBackendError> {
        drop(self);
        Ok(())
    }
}

#[async_trait]
impl DbBackend for MockDb {
    async fn new() -> Result<Self, DbBackendError> {
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

    async fn close(self) -> Result<(), DbBackendError> {
        drop(self); //XXX: check how RwLocks are dropped, could be problematic?
        Ok(())
    }
}

#[async_trait]
impl<T: DbBackend + Send + Sync> DbBackend for Data<T> {
    async fn new() -> Result<Data<T>, DbBackendError> {
        Ok(Data::new(T::new().await?))
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
    async fn new() -> Result<Arc<T>, DbBackendError> {
        Ok(Arc::new(T::new().await?))
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

//XXX: this could be refactored to export internal types? Would be better for error handling
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbBackendError {
    AlreadyExist,
    NoConnectionAvailable(String),
    QueryFailed(String),
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
