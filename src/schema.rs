#![allow(clippy::all, missing_docs)]

table! {
    agents (id) {
        id -> Integer,
        public_id -> BigInt,
        secure_key -> Binary,
        last_seen -> BigInt,
    }
}
