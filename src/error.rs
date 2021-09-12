//! Contains the error type used by this application
//! which wraps many different types of suberrors, so we can return a consistent type.

use mobc_postgres::tokio_postgres;
use serde::Serialize;
use std::convert::Infallible;
use std::fmt::{self, Formatter};
use warp::hyper::StatusCode;
use warp::{Rejection, Reply};

pub enum Error {
    DBPool(mobc::Error<tokio_postgres::Error>),
    DBQuery(tokio_postgres::Error),
    DBInit(tokio_postgres::Error),
    ReadFile(std::io::Error),
    NoStream,
    StreamTimeout,
    #[allow(dead_code)]
    Stream(tokio::sync::oneshot::error::RecvError),
    Server(String),
    #[allow(dead_code)]
    Message(String),
}

impl warp::reject::Reject for Error {}

impl std::convert::From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::ReadFile(e)
    }
}

pub async fn handle_rejection(err: Rejection) -> std::result::Result<impl Reply, Infallible> {
    let code;
    let message;

    //Todo could replace with match statement using request guards
    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "Not Found";
    } else if err.find::<warp::filters::body::BodyDeserializeError>().is_some() {
        code = StatusCode::BAD_REQUEST;
        message = "Invalid Body";
    } else if let Some(e) = err.find::<Error>() {
        match e {
            Error::DBQuery(_) => {
                code = StatusCode::BAD_REQUEST;
                message = "Could not Execute request";
            }
            _ => {
                eprintln!("unhandled application error: {:?}", err);
                code = StatusCode::INTERNAL_SERVER_ERROR;
                message = "Internal Server Error";
            }
        }
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        code = StatusCode::METHOD_NOT_ALLOWED;
        message = "Method Not Allowed";
    } else {
        eprintln!("unhandled error: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal Server Error";
    }

    let json = warp::reply::json(&ErrorMessage {
        message: message.to_owned(),
    });
    Ok(warp::reply::with_status(json, code))
}

#[derive(Debug, Serialize)]
struct ErrorMessage {
    message: String,
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::DBInit(e) => f.write_str(&e.to_string()),
            _ => f.write_str("Server error occured"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str("Server error occured")
        // match &*self {
        //     Error::NotFoundError => f.write_str("Unable to contact server."),
        // }
    }
}
