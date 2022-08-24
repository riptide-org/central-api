#![allow(clippy::extra_unused_lifetimes, unused_qualifications)]

use crate::schema::*;

#[derive(Debug, Insertable)]
#[table_name = "agents"]
pub struct NewAgent {
    pub public_id: i64,
    pub secure_key: Vec<u8>,
}

#[derive(Queryable)]
pub struct Agent {
    pub id: i32,
    pub public_id: i64,
    pub secure_key: Vec<u8>,
}
