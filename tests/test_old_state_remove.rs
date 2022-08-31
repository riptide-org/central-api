use log::{error, info};
use ntest::timeout;

mod common;

use common::{create_server, find_open_port, init_logger, AuthToken};
use tungstenite::Message;
use ws_com_framework::error::ErrorKind;

#[actix_web::test]
#[timeout(15_000)]
async fn test_old_unauth_servers_removed() {
    init_logger();

    let database_url = String::from("./test-db-test-old-unauth-servers-removed.db");
    let port = find_open_port();
    let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
    let (_, state, handle, tx) = create_server(database_url.clone(), port).await;

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

    // validate that we are in the state as unauthenticated
    assert!(state
        .unauthenticated_servers
        .read()
        .await
        .contains_key(&token.public_id));

    // wait 5 seconds
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;

    // validate that the websocket has closed
    let mut msg = socket.read_message().unwrap();

    //skip non-ping messages
    if msg.is_ping() {
        msg = socket.read_message().unwrap();
    }

    assert!(msg.is_close());

    // validate that we are no longer in the state as unauthenticated
    assert!(!state
        .unauthenticated_servers
        .read()
        .await
        .contains_key(&token.public_id));

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[timeout(15_000)]
async fn test_old_requests_removed() {
    init_logger();

    let database_url = String::from("./test-db-test-old-requests-removed.db");
    let port = find_open_port();
    let address = format!("127.0.0.1:{}", port.local_addr().unwrap().port());
    let (_, state, handle, tx) = create_server(database_url.clone(), port).await;

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

    // send auth response
    let auth_res = ws_com_framework::Message::AuthRes {
        public_id: token.public_id,
        passcode: token.passcode.into(),
    };
    socket
        .write_message(tungstenite::Message::Binary(auth_res.try_into().unwrap()))
        .unwrap();

    // validate OK response
    let msg = socket.read_message().unwrap();
    let expected_data: Vec<u8> = ws_com_framework::Message::Ok.try_into().unwrap();
    assert_eq!(msg.into_data(), expected_data);

    // reuse client to send file request to server
    let get_url = format!("http://{}/agents/{}/files/{}", address, token.public_id, 24);
    let res = tokio::task::spawn(async move {
        let res = client.get(get_url).send().await.unwrap();

        info!("got response {:?}", res);
        assert_eq!(res.status(), 404);

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

    //validate that data is an uploadRequest variant
    let upload_id: u64;
    let converted_upload_url: String;
    let data: ws_com_framework::Message = msg.into_data().try_into().unwrap();
    if let ws_com_framework::Message::UploadTo {
        file_id: _,
        upload_url,
    } = data
    {
        converted_upload_url = upload_url
            .replace("localhost:8080", &address)
            .replace("https", "http");

        // validate that there is a waiting request in the state
        upload_id = upload_url
            .split('/')
            .last()
            .unwrap()
            .to_string()
            .parse()
            .unwrap();

        assert!(state.requests.read().await.contains_key(&upload_id));
    } else {
        panic!("expected UploadRequest variant");
    }

    // wait 5 seconds
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // validate that the request returned with 404 error
    let res = res.await.unwrap();
    assert!(res.status().is_client_error());

    // validate request no longer in server state
    assert!(!state.requests.read().await.contains_key(&upload_id));

    // validate that the server got an error message from the server
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

    // // check error response
    let data: ws_com_framework::Message = msg.into_data().try_into().unwrap();
    let expected_data: ws_com_framework::Message = ws_com_framework::Message::Error {
        kind: ErrorKind::FailedFileUpload,
        reason: Some(String::from("failed to upload in time")),
    };
    assert_eq!(data, expected_data);

    // trying to upload the file now should fail with 404 not found
    let client = reqwest::Client::new();
    let res = client
        .post(converted_upload_url)
        .body("hello, world")
        .send()
        .await
        .unwrap();

    // validate is 404
    assert_eq!(res.status(), 404);

    //kill the std thread without waiting
    if let Err(e) = tx.send(()) {
        error!("{:?}", e);
    }
    handle.join().unwrap();

    // remove database file
    std::fs::remove_file(database_url).unwrap();
}
