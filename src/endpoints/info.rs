//! Handles the information/data acquisition side of the API

use actix_web::{
    get,
    web::{self, Data, Path},
    HttpResponse,
};
use log::{error, trace};
use rand::Rng;
use serde::Serialize;
use tokio::sync::mpsc;
use ws_com_framework::{Message, PublicId};

use crate::{
    config::Config,
    endpoints::{util::collect_bytes, websockets::InternalComm},
    State,
};

/// configure all information endpoint services
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(global_info).service(agents_info);
}

#[derive(Serialize)]
struct InfoResp<'r> {
    ready: bool,
    message: &'r str,
    uptime: u64,
}

/// endpoint which returns information about the api (GET /info)
#[get("/info")]
async fn global_info(state: Data<State>) -> impl actix_web::Responder {
    web::Json(InfoResp {
        ready: true,
        message: "ready",
        uptime: state.start_time.elapsed().as_secs(),
    })
}

//TODO: update ws_com_framework to support this properly, as we need to send a "STATUS" request message to the user and expect a response
/// endpoint which returns information about all a specific agent connected to the api
#[get("/agents/{agent_id}")]
async fn agents_info(
    state: Data<State>,
    agent_id: Path<PublicId>,
    config: Data<Config>,
) -> HttpResponse {
    let agent_id = agent_id.into_inner();

    let reader = state.servers.read().await;
    let agent = reader.get(&agent_id);

    if agent.is_none() {
        return HttpResponse::NotFound().json(InfoResp {
            ready: false,
            message: "agent not found, or not authenticated",
            uptime: 0,
        });
    }

    //XXX: Below is duplicate of metadata endpoint, and should be refactored into a function
    trace!("agent {} found, sending status request", agent_id);

    let (tx, mut rx) = mpsc::channel(100);
    let download_id = rand::thread_rng().gen();

    trace!("Download id is {}", download_id);

    //create the upload job
    state.requests.write().await.insert(download_id, tx);

    //attach channel to ws, and send upload req. to server
    let connected_servers = state.servers.read().await;
    let uploader_ws = connected_servers.get(&agent_id).unwrap();
    uploader_ws
        .send(InternalComm::SendMessage(Message::StatusReq {
            public_id: agent_id,
            upload_id: download_id,
        }))
        .await
        .unwrap();

    trace!("generating response payload and returning");

    let payload = match collect_bytes(
        &download_id,
        &mut rx,
        config.request_timeout_seconds,
        &state,
        uploader_ws,
        &agent_id,
        Some(2_000_000),
    )
    .await
    {
        Ok(p) => p,
        Err(e) => return e,
    };

    let payload: String = match String::from_utf8(payload) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to convert payload to string: {}", e);
            return HttpResponse::InternalServerError().json(InfoResp {
                ready: false,
                message: "failed to convert payload to string",
                uptime: 0,
            });
        }
    };

    //create a response
    HttpResponse::Ok()
        .content_type("application/json")
        .body(payload)
}
