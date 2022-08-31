//! Handles the information/data acquisition side of the API

use actix_web::{
    get,
    web::{self, Path},
    HttpResponse,
};
use log::{error, trace};
use rand::Rng;
use serde::Serialize;
use tokio::sync::mpsc;
use ws_com_framework::{Message, PublicId};

use crate::{endpoints::websockets::InternalComm, State};

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
async fn global_info(state: web::Data<State>) -> impl actix_web::Responder {
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
    state: web::Data<State>,
    agent_id: Path<PublicId>,
) -> impl actix_web::Responder {
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

    let mut response: Vec<u8> = Vec::with_capacity(100);
    while let Some(s) = rx.recv().await {
        match s {
            Ok(b) => {
                response.extend(b.into_iter());

                // if response size is greater than 2mb, return an error
                if response.len() > 2_000_000 {
                    error!("response size is greater than 2mb on request to agent {}, current body is `{}`", agent_id, String::from_utf8_lossy(&response));
                    // reduce the body to the first 2mb
                    response.truncate(2_000_000);
                    break;
                }
            }
            Err(e) => {
                error!("Error while receiving data from server: {}", e);
                return HttpResponse::InternalServerError().finish();
            }
        }
    }

    //create a streaming response
    HttpResponse::Ok()
        .content_type("application/json")
        .body(response)
}
