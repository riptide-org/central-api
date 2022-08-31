#![cfg(not(tarpaulin_include))]

mod common;

use central_api_lib::db::DbBackend;
use common::{create_server, find_open_port, init_logger, AuthToken};
use log::error;
use ntest::timeout;
use tungstenite::Message;

/// Test the websocket authentication process
#[actix_web::test]
#[timeout(20_000)]
async fn test_websocket_auth() {
    init_logger();
    let database_url = String::from("./test-db-test-websocket-auth.db");
    let port = find_open_port();
    let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
    let (db, state, handle, tx) = create_server(database_url.clone(), port).await;

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

    //validate that token is in the database
    assert!(db.contains_entry(&token.public_id).await.unwrap());

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

    // check that the state contains us as an entry in unauthenticated_servers
    assert!(state
        .unauthenticated_servers
        .read()
        .await
        .contains_key(&token.public_id));

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

    // check that the state no longer contains us as an entry in unauthenticated_servers
    assert!(!state
        .unauthenticated_servers
        .read()
        .await
        .contains_key(&token.public_id));

    // check that the state contains us as an entry in servers
    assert!(state.servers.read().await.contains_key(&token.public_id));

    // test ping/pong
    socket.write_message(Message::Ping(vec![1, 2, 3])).unwrap();
    let msg = socket.read_message().unwrap();
    assert_eq!(msg, Message::Pong(vec![1, 2, 3]));

    // test close
    socket.write_message(Message::Close(None)).unwrap();

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}
