use serde::{Serialize, Deserialize};
use chrono::prelude::*;
use crate::error::Error;
use mobc_postgres::tokio_postgres::Row;
use std::convert::TryFrom;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Agent {
    id: i64,
    created_at: DateTime<Utc>, //TODO, do we even need to write this to the db? We could have serde skip it.
    last_signin: DateTime<Utc>,
    unique_id: String,
}

impl Agent {
    pub fn id(&self) -> i64 {
        self.id
    }
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&format!("id: {}, unique_id: {}", self.id, self.unique_id))
    }
}

impl crate::db::FromDataBase for Agent {
    type Error = Error;
    fn from_database(row: &Row) -> Result<Agent, Error> {
        Ok(Agent {
            id: row.get(0),
            created_at: row.get(1),
            last_signin: row.get(2),
            unique_id: row.get(3),
        })
    }
}

#[derive(Deserialize)]
pub struct AgentRequest {
    unique_id: String,
}

impl AgentRequest {
    pub fn unique_id(&self) -> &str {
        &self.unique_id
    }
    pub fn new(unique_id: String) -> Self {
        AgentRequest { unique_id }
    }
}

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

#[derive(Serialize)]
pub enum MessageResponse {
    Error(String),
    Message(String),
    Created(String),
}

impl std::convert::TryFrom<String> for MessageResponse {
    type Error = Error;
    fn try_from(s: String) -> Result<MessageResponse, Error> {
        let mut s = s.split(" ").collect::<Vec<&str>>();
        let t: &str = s.remove(0);
        return match t {
            "Error" => Ok(MessageResponse::Error(s.join(""))),
            "Message" => Ok(MessageResponse::Message(s.join(""))),
            "Created" => Ok(MessageResponse::Created(s.join(""))),
            _ => Err(format!("Unexpected message response recieved! Response: {} {:?}", t, s)),
        }
    }
}