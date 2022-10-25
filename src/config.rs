//! Handle configuration of the central server.

use std::{
    env,
    net::{Ipv4Addr, TcpListener},
};

use dotenv::dotenv;

/// Handles all configuration variables for the crate
#[derive(Debug)]
pub struct Config {
    /// The listener to bind to
    pub listener: TcpListener,
    /// The database url to connect to
    pub db_url: String,
    /// The base URL of this server
    pub base_url: String,
    /// Time before an authentication request will timeout and return a failure to a potential client (in seconds)
    pub auth_timeout_seconds: u64,
    /// Time before an authentication request will timeout and return a failure for a request (in seconds)
    pub request_timeout_seconds: u64,
    /// Number of seconds between pings to a server
    pub ping_interval: u64,
    /// The PSK that new clients can use to authenticate during registration
    pub password: Option<String>,
}

impl Config {
    /// Load configuration from the environment
    pub fn load() -> Result<Config, String> {
        dotenv().ok();

        let host: Ipv4Addr = env::var("HOST")
            .map_err(|_| "HOST must be set")?
            .parse()
            .map_err(|_| "HOST must be a valid IPv4 address")?;

        let port: u16 = env::var("PORT")
            .map_err(|_| "PORT must be set")?
            .parse()
            .map_err(|_| "PORT must be valid IPv4 address")?;

        let listener = TcpListener::bind((host, port))
            .map_err(|e| format!("failed to bind to {}:{} due to error: {}", host, port, e))?;

        Ok(Self {
            listener,
            db_url: env::var("DB_URL").map_err(|_| "DB_URL must be set")?,
            base_url: env::var("BASE_URL").map_err(|_| "BASE_URL must be set")?,
            request_timeout_seconds: env::var("REQUEST_TIMEOUT")
                .map_err(|_| "REQUEST_TIMEOUT must be set")?
                .parse()
                .map_err(|_| "REQUEST_TIMEOUT must be 64-bit integer")?,
            auth_timeout_seconds: env::var("AUTH_TIMEOUT")
                .map_err(|_| "AUTH_TIMEOUT must be set")?
                .parse()
                .map_err(|_| "AUTH_TIMEOUT must be 64-bit integer")?,
            ping_interval: env::var("PING_INTERVAL")
                .map_err(|_| "PING_INTERVAL must be set")?
                .parse()
                .map_err(|_| "PING_INTERVAL must be 64-bit integer")?,
            password: env::var("PASSWORD").ok(),
        })
    }
}
