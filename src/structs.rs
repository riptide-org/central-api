use crate::error::Error;
use chrono::prelude::*;
use mobc_postgres::tokio_postgres::Row;
use serde::{Deserialize, Serialize};

/// A valid server agent, and all relevant details thereof.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Agent {
    public_id: String,
    created_at: DateTime<Utc>, //TODO, do we even need to write this to the db? We could have serde skip it.
    last_signin: DateTime<Utc>,
    secure_key_hashed: String,
}

impl Agent {
    pub fn public_id(&self) -> &str {
        &self.public_id
    }

    pub fn secure_key_hashed(&self) -> &str {
        &self.secure_key_hashed
    }
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&format!(
            "public id: {}, secure_key: {:?}",
            self.public_id, self.secure_key_hashed
        ))
    }
}

impl crate::db::FromDataBase for Agent {
    type Error = Error;
    fn from_database(row: &Row) -> Result<Agent, Error> {
        Ok(Agent {
            public_id: row.get(1),
            created_at: row.get(2),
            last_signin: row.get(3),
            secure_key_hashed: row.get(4),
        })
    }
}

/// An agent request, used to create a new agent.
/// Note that these types are stored as hexadecimal converted to a
pub struct AgentRequest {
    secure_key: String,
    public_id: String,
}

impl AgentRequest {
    pub fn secure_key_plain(&self) -> &str {
        &self.secure_key
    }

    pub fn secure_key_hashed(&self) -> String {
        crate::common::hash(&self.secure_key)
    }

    pub fn public_id(&self) -> &str {
        &self.public_id
    }
}

impl Default for AgentRequest {
    fn default() -> AgentRequest {
        let public_id: String = crate::common::get_random_hex(6);
        let secure_key = crate::common::get_random_hex(32);

        AgentRequest {
            secure_key,
            public_id,
        }
    }
}

/// A request to the database, for updating purposes only.
#[derive(Deserialize)]
pub struct AgentUpdateRequest {
    last_signin: DateTime<Utc>,
}

impl AgentUpdateRequest {
    pub fn new(last_signin: DateTime<Utc>) -> Self {
        AgentUpdateRequest { last_signin }
    }

    pub fn last_signin(&self) -> DateTime<Utc> {
        self.last_signin
    }
}
