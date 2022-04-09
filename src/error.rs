use crate::db::DbBackendError;

#[derive(Debug, Eq, PartialEq)]
pub enum HttpError {

}

impl From<DbBackendError> for HttpError {
    fn from(_: DbBackendError) -> Self {
        todo!()
    }
}

impl From<HttpError> for actix_web::Error {
    fn from(_: HttpError) -> Self {
        todo!()
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for HttpError {

}