use std::{ops::Deref, collections::HashMap, rc::Rc, sync::Arc};

use actix_web::web::Data;
use async_trait::async_trait;
use diesel::{SqliteConnection, r2d2::{ConnectionManager, self}};
use log::trace;
use tokio::sync::RwLock;
use ws_com_framework::Passcode;

use crate::ServerId;

type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

#[derive(Clone)]
pub struct Database {
    pool: DbPool,
}

/// An entirely insecure mock implementation of a backend database for development and testing purposes
pub struct MockDb {
    store: RwLock<HashMap<ServerId, Passcode>>,
}

#[async_trait]
pub trait DbBackend {
    /// `new` will attempt
    async fn new() -> Result<Self, DbBackendError> where Self: Sized;

    /// `save_entry` will take a passcode and `ServerId` and save it into the server.
    async fn save_entry(&self, server_id: ServerId, passcode: Passcode) -> Result<Option<Passcode>, DbBackendError>;

    /// `validate_server` will take a provided `ServerId` and return true if the provided passcode matches.
    /// It will return false if the provided passcode fails validation.
    async fn validate_server(&self, server_id: &ServerId, passcode: &Passcode) -> Result<bool, DbBackendError>;

    /// `contains_entry` will validate whether the provided `ServerId` exists in the database
    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError>;

    /// `close` will attempt to destroy this connection to the database
    async fn close(self) -> Result<(), DbBackendError>;
}

#[async_trait]
impl DbBackend for Database {
    async fn new() -> Result<Self, DbBackendError> {
        todo!()
    }

    async fn save_entry(&self, server_id: ServerId, passcode: Passcode) -> Result<Option<Passcode>, DbBackendError> {
        todo!()
    }

    async fn validate_server(&self, server_id: &ServerId, passcode: &Passcode) -> Result<bool, DbBackendError> {
        todo!()
    }

    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
        todo!()
    }

    async fn close(self) -> Result<(), DbBackendError> {
        todo!()
    }
}

#[async_trait]
impl DbBackend for MockDb {
    async fn new() -> Result<Self, DbBackendError> {
        Ok(Self {
            store: Default::default()
        })
    }

    async fn save_entry(&self, server_id: ServerId, passcode: Passcode) -> Result<Option<Passcode>, DbBackendError> {
        Ok(self.store.write().await.insert(server_id, passcode))
    }

    async fn validate_server(&self, server_id: &ServerId, passcode: &Passcode) -> Result<bool, DbBackendError> {
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

    async fn save_entry(&self, server_id: ServerId, passcode: Passcode) -> Result<Option<Passcode>, DbBackendError> {
        self.deref().save_entry(server_id, passcode).await
    }

    async fn validate_server(&self, server_id: &ServerId, passcode: &Passcode) -> Result<bool, DbBackendError> {
        self.deref().validate_server(server_id, passcode).await
    }

    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
        self.deref().contains_entry(server_id).await
    }

    async fn close(self) -> Result<(), DbBackendError> {
        match std::sync::Arc:: <T> ::try_unwrap(self.into_inner()) {
            Ok(r) => r.close().await,
            Err(_) => {
                trace!("attempted closure with more than 1 strong reference");
                Ok(())
            },
        }
    }
}

#[async_trait]
impl<T: DbBackend + Send + Sync> DbBackend for Arc<T> {
    async fn new() -> Result<Arc<T>, DbBackendError> {
        Ok(Arc::new(T::new().await?))
    }

    async fn save_entry(&self, server_id: ServerId, passcode: Passcode) -> Result<Option<Passcode>, DbBackendError> {
        self.deref().save_entry(server_id, passcode).await
    }

    async fn validate_server(&self, server_id: &ServerId, passcode: &Passcode) -> Result<bool, DbBackendError> {
        self.deref().validate_server(server_id, passcode).await
    }

    async fn contains_entry(&self, server_id: &ServerId) -> Result<bool, DbBackendError> {
        self.deref().contains_entry(server_id).await
    }

    async fn close(self) -> Result<(), DbBackendError> {
        match std::sync::Arc:: <T> ::try_unwrap(self) {
            Ok(r) => r.close().await,
            Err(_) => {
                trace!("attempted closure with more than 1 strong reference");
                Ok(())
            },
        }
    }
}

#[derive(Debug)]
pub enum DbBackendError {
    AlreadyExist
}

impl std::fmt::Display for DbBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for DbBackendError {

}