/*
    Author: Josiah Bull

    This application is the central api of the file sharing system. 
    It aims to fufill the following basic spec: 
    - Accept websocket connections from server agent
        - Should issue a persistent ID for that server agent.
        - Ext, fingerprint the server agent in some way and store that data?
    - Serve the front end static information pages
    - Accept GET resquests to download a file
        - Should wait for paired POST
        - Check if the server is online
        - Support timeouts
    - Accept POST requests to upload a file
        - Supports paired get request
        - Gracefully handle failed/interrupted upload

    - Ext: opt-out webrtc for proper peer-to-peer?
*/

use warp::{path, Filter};

#[tokio::main]
async fn main() {

    let ws = warp::path("server-register")
        .and(path::end())
        .and(warp::ws())
        .map(|socket: warp::ws::Ws | warp::reply::reply());

    let upload = warp::post()
        .and(warp::path("upload"))
        .and(path::param())
        .and(path::end())
        .map(|id: usize | warp::reply::reply());
    
    let download = warp::get()
        .and(warp::path("download"))
        .and(path::param())
        .and(path::param())
        .and(path::end())
        .map(|server_id: usize, file_id: String| warp::reply::reply());

    //Non-Api requests, this will be like the front end website etc
    let root = warp::any()
        .map(|| warp::reply::reply());

    let routes = ws.or(upload).or(download).or(root).with(warp::cors().allow_any_origin());

    warp::serve(routes)
        .run(([127, 0, 0, 1], 3030))
        .await;
}