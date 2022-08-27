#![cfg(not(tarpaulin_include))]

mod common;

use common::{create_server, find_open_port, init_logger, AuthToken};
use log::{error, info};
use ntest::timeout;
use serde::Deserialize;
use tungstenite::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[timeout(20_000)]
async fn test_status_request() {
    init_logger();
    let database_url = String::from("./test-db-test-status-request.db");
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

    let token = res.json::<AuthToken>().await.unwrap();

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

    //send auth message to server
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

    // reuse client to send status request to the server
    let get_url = format!("http://{}/agents/{}", address, token.public_id);
    let res = tokio::task::spawn(async move {
        let res = client.get(get_url).send().await.unwrap();

        info!("got response {:?}", res);
        assert!(res.status().is_success());

        res
    });

    // read messages until we get a binary message type
    let mut msg: Message = socket.read_message().unwrap();
    while !matches!(msg, Message::Binary(_)) {
        info!("got message: {:?}", msg);
        (msg, socket) = tokio::task::spawn_blocking(move || {
            let msg = socket.read_message().unwrap();
            (msg, socket)
        })
        .await
        .unwrap();
    }

    let data: ws_com_framework::Message = msg.into_data().try_into().unwrap();
    match data {
        ws_com_framework::Message::StatusReq {
            public_id,
            upload_id,
        } => {
            //send status response to server
            socket
                .write_message(Message::Binary(
                    ws_com_framework::Message::StatusRes {
                        public_id,
                        ready: true,
                        uptime: 1230,
                        upload_id,
                        message: Some(String::from("hello, world - I'm alive!")),
                    }
                    .try_into()
                    .unwrap(),
                ))
                .unwrap();
        }
        m => panic!("expected status req message {:?}", m),
    }

    // validate that the status was received
    let got_res = res.await.unwrap();
    assert!(got_res.status().is_success());

    //validate that content type header is present and set to application/json
    let content_type = got_res.headers().get("content-type").unwrap();
    assert_eq!(content_type, "application/json");

    #[derive(Debug, Deserialize)]
    struct AgentStatus {
        ready: bool,
        uptime: u64,
        message: String,
    }

    let got_res = got_res.json::<AgentStatus>().await.unwrap();
    assert!(got_res.ready);
    assert_eq!(got_res.uptime, 1230);
    assert_eq!(got_res.message, "hello, world - I'm alive!");

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}
