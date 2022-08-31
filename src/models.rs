//! Models that can be used throughout the application for inserting into the database, or for use in the API as request/response types.

#![allow(clippy::extra_unused_lifetimes, unused_qualifications)]

use crate::schema::*;

/// A model representing a `NewAgent` to be inserted into the database
#[derive(Debug, Insertable)]
#[diesel(table_name = agents)]
pub struct NewAgent {
    /// The public ID representing this agent
    pub public_id: i64,
    /// The hashed secure key that this agent must login with
    pub secure_key: Vec<u8>,
    /// When this user was last seen
    pub last_seen: i64,
}

/// A model representing an `Agent` from the database
#[derive(Debug, Queryable)]
pub struct Agent {
    /// The ID of this agent in the database
    pub id: i32,
    /// The public ID representing this agent
    pub public_id: i64,
    /// The hashed secure key that this agent must login with
    pub secure_key: Vec<u8>,
    /// When this user was last seen
    pub last_seen: i64,
}
