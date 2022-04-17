use crate::db::DbBackendError;

#[derive(Debug, Eq, PartialEq)]
pub enum HttpError {
    Other(String), //TODO
}

impl From<DbBackendError> for HttpError {
    fn from(e: DbBackendError) -> Self {
        Self::Other(e.to_string())
    }
}

impl From<HttpError> for actix_web::Error {
    fn from(_: HttpError) -> Self {
        todo!()
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Other(e) => write!(f, "an error occured: {}", e),
        }
    }
}

impl std::error::Error for HttpError {}
