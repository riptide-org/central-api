#![cfg(not(tarpaulin_include))]

mod common;

use common::{create_server, find_open_port, init_logger, AuthToken};
use log::error;
use ntest::timeout;
use tungstenite::Message;
use ws_com_framework::error::ErrorKind;

/// Test logging in with a valid public_id, but invalid passcode
#[actix_web::test]
#[timeout(20_000)]
async fn test_websocket_auth_invalid_login() {
    init_logger();
    let database_url = String::from("./test-db-test-websocket-auth-failure.db");
    let port = find_open_port();
    let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
    let (_, _, handle, tx) = create_server(database_url.clone(), port).await;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // make POST request to http endpoint at /register to get a token
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/register", address))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());

    let token: AuthToken = res.json().await.unwrap();

    // create a websocket connection to the server
    let (mut socket, _) =
        tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap();

    let msg = socket.read_message().unwrap();
    let expected_data: Vec<u8> = ws_com_framework::Message::AuthReq {
        public_id: token.public_id,
    }
    .try_into()
    .unwrap();
    assert_eq!(msg.into_data(), expected_data);

    // send auth message with invalid passcode
    socket
        .write_message(Message::Binary(
            ws_com_framework::Message::AuthRes {
                public_id: token.public_id,
                passcode: "invalid".into(),
            }
            .try_into()
            .unwrap(),
        ))
        .unwrap();

    //validate we got error response
    let msg = socket.read_message().unwrap();
    let expected_data: Vec<u8> = ws_com_framework::Message::Error {
        reason: Some("failed authentication".into()),
        kind: ErrorKind::InvalidSession,
    }
    .try_into()
    .unwrap();
    assert_eq!(msg.into_data(), expected_data);

    // validate that the socket was closed
    let msg = socket.read_message().unwrap();
    assert_eq!(msg, Message::Close(None));

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}

/// Test that trying to login with a completely invalid public_id and passcode
#[actix_web::test]
#[timeout(20_000)]
async fn test_websocket_auth_invalid_public_id() {
    init_logger();
    let databse_url = String::from("./test-db-test-websocket-auth-failure-id.db");
    let port = find_open_port();
    let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
    let (_, _, handle, tx) = create_server(databse_url.clone(), port).await;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // create a websocket connection to the server
    let err = tungstenite::connect(format!("ws://{}/ws/{}", address, "invalid")).unwrap_err();

    // check that err was 404
    assert!(err.to_string().contains("404 Not Found"));

    // kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(databse_url).unwrap();
}

/// This tests that a user trying to login twice will fail on the second login.
/// Then, if the first login is closed - a new login can succeed.
#[actix_web::test]
#[timeout(20_000)]
async fn test_websocket_auth_double_login() {
    init_logger();
    let database_url = String::from("./test-db-test-websocket-auth-failure-double.db");
    let port = find_open_port();
    let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());

    let (_, _, handle, tx) = create_server(database_url.clone(), port).await;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // make POST request to http endpoint at /register to get a token
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/register", address))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());

    let token: AuthToken = res.json().await.unwrap();

    // create a websocket connection to the server
    let (mut socket, _) =
        tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap();

    let msg = socket.read_message().unwrap();
    let expected_bytes: Vec<u8> = ws_com_framework::Message::AuthReq {
        public_id: token.public_id,
    }
    .try_into()
    .unwrap();
    assert_eq!(msg.into_data(), expected_bytes);

    // send auth message to the server
    socket
        .write_message(Message::Binary(
            ws_com_framework::Message::AuthRes {
                public_id: token.public_id,
                passcode: token.passcode.into(),
            }
            .try_into()
            .unwrap(),
        ))
        .unwrap();

    // expect OK response
    let msg = socket.read_message().unwrap();
    let expected_bytes: Vec<u8> = ws_com_framework::Message::Ok.try_into().unwrap();
    assert_eq!(msg.into_data(), expected_bytes);

    // try to login again with a new socket
    let err = tungstenite::connect(format!("ws://{}/ws/{}", address, token.public_id)).unwrap_err();

    match err {
        tungstenite::Error::Http(e) => {
            assert_eq!(e.status(), 403);

            let body: String = e.into_body().expect("body attached to response");
            assert!(body.contains("already authenticated with this id"));
        }
        m => panic!("expected http error: `{:?}`", m),
    }

    // close the first socket
    socket.close(None).unwrap();

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}
