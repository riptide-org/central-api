use std::convert::Infallible;
use std::fmt::{Formatter, self};
use warp::{Reply, Rejection};
use serde::{ Serialize};
use warp::hyper::{Response as Builder, StatusCode, Body};
use mobc_postgres::tokio_postgres;

pub enum Error {
    DBPoolError(mobc::Error<tokio_postgres::Error>),
    DBQueryError(tokio_postgres::Error),
    DBInitError(tokio_postgres::Error),
    ReadFileError(std::io::Error),
    NoStream,
    StreamTimeout,
    StreamError(tokio::sync::oneshot::error::RecvError),
    ServerError(String),
}

impl warp::reject::Reject for Error {}

#[derive(Serialize)]
struct ErrorResponse {
    message: String,
}

pub async fn handle_rejection(err: Rejection) -> std::result::Result<impl Reply, Infallible> {
    let code;
    let message;

    //Todo could replace with match statement using request guards
    if err.is_not_found() {
            code = StatusCode::NOT_FOUND;
            message = "Not Found";
    } else if let Some(_) = err.find::<warp::filters::body::BodyDeserializeError>() {
            code = StatusCode::BAD_REQUEST;
            message = "Invalid Body";
    } else if let Some(e) = err.find::<Error>() {
            match e {
                Error::DBQueryError(_) => {
                    code = StatusCode::BAD_REQUEST;
                    message = "Could not Execute request";
                }
                _ => {
                    eprintln!("unhandled application error: {:?}", err);
                    code = StatusCode::INTERNAL_SERVER_ERROR;
                    message = "Internal Server Error";
                }
            }
    } else if let Some(_) = err.find::<warp::reject::MethodNotAllowed>() {
            code = StatusCode::METHOD_NOT_ALLOWED;
            message = "Method Not Allowed";
    } else {
            eprintln!("unhandled error: {:?}", err);
            code = StatusCode::INTERNAL_SERVER_ERROR;
            message = "Internal Server Error";
    }

    let json = warp::reply::json(&ErrorResponse {
            message: message.into(),
    });

    Ok(warp::reply::with_status(json, code))
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("Server error occured")
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