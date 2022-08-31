mod common;

use common::*;
use log::{error, info};
use ntest::timeout;
use tungstenite::Message;
use ws_com_framework::error::ErrorKind;

/// validate that failing to respond to pings will lead to the client being ejected
#[actix_web::test]
#[timeout(15_000)]
async fn test_failed_ping_kicked() {
    init_logger();

    let database_url = String::from("./test-db-test-failed-pings-removed.db");
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

    // send authentication message
    let auth_msg = ws_com_framework::Message::AuthRes {
        public_id: token.public_id,
        passcode: token.passcode.into(),
    };
    socket
        .write_message(Message::Binary(auth_msg.try_into().unwrap()))
        .unwrap();

    //validate ok response
    let msg = socket.read_message().unwrap();
    let expected_data: Vec<u8> = ws_com_framework::Message::Ok.try_into().unwrap();
    assert_eq!(msg.into_data(), expected_data);

    // wait 6 seconds, then read incoming messages
    // tokio::time::sleep(std::time::Duration::from_secs(6)).await;

    // we should have received 2 ping messages and an error message
    let mut ping_count = 0;
    let mut error_count = 0;

    loop {
        let msg = socket.read_message().unwrap();
        if msg.is_close() {
            info!("got close message");

            //XXX: check close has reason attached?

            break;
        }
        if msg.is_ping() {
            // tungstenite will automatically attempt to send a ping response
            // we want to override that response and prevent it from being sent, instead replacing it with garbage data
            socket.write_message(Message::Pong(vec![255])).unwrap();
            ping_count += 1;
            info!("got ping message");
        }
        if msg.is_binary() {
            info!("got binary message");
            let msg: ws_com_framework::Message =
                ws_com_framework::Message::try_from(msg.into_data()).unwrap();
            if let ws_com_framework::Message::Error { kind, reason } = msg {
                assert!(matches!(kind, ErrorKind::Unknown)); //XXX: we should add ping failure to error type
                let reason = reason.unwrap();

                info!("reason: {:?}", reason);

                assert!(reason.contains("failed to respond to ping"));
            }
            error_count += 1;
        }
    }

    assert_eq!(ping_count, 1);
    assert_eq!(error_count, 1);

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}
