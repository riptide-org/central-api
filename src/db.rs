use crate::ServerId;

#[derive(Debug, Clone, Copy)]
pub struct MockDB {

}

impl DbBackend for MockDB {
    fn save_entry(server_id: ServerId, passcode: [u8; 32]) -> Result<(), DbBackendError> where Self:Sized {
        todo!()
    }

    fn validate_server(server_id: ServerId, passcode: &[u8; 32]) -> Result<bool, DbBackendError> where Self:Sized {
        todo!()
    }

    fn contains_entry(server_id: ServerId) -> Result<bool, DbBackendError> where Self:Sized {
        todo!()
    }
}

pub trait DbBackend: Clone {
    fn save_entry(server_id: ServerId, passcode: [u8; 32]) -> Result<(), DbBackendError> where Self:Sized;
    fn validate_server(server_id: ServerId, passcode: &[u8; 32]) -> Result<bool, DbBackendError> where Self:Sized;
    fn contains_entry(server_id: ServerId) -> Result<bool, DbBackendError> where Self:Sized;
}

#[derive(Debug)]
pub enum DbBackendError {

}