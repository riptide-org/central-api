use actix_web::{web::{self, Path}, get};
use serde::Serialize;
use ws_com_framework::PublicId;

use crate::State;

/// configure all information endpoint services
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg
        .service(global_info)
        .service(agents_info);
}

#[derive(Serialize)]
struct InfoResp<'r> {
    ready: bool,
    message: &'r str,
    uptime: u64,
}

/// endpoint which returns information about the api (GET /info)
#[get("/info")]
async fn global_info(state: web::Data<State>,) -> impl actix_web::Responder {
    web::Json(InfoResp {
        ready: true,
        message: "ready",
        uptime: state.start_time.elapsed().as_secs(),
    })
}


#[derive(Serialize)]
struct AagentInfo<'r> {
    ready: bool,
    message: &'r str,
    // uptime: u64,
    authenticated: bool,
}

//TODO: update ws_com_framework to support this properly, as we need to send a "STATUS" request message to the user and expect a response
/// endpoint which returns information about all a specific agent connected to the api
#[get("/agents/{agent_id}")]
async fn agents_info(agent_id: Path<PublicId>, state: web::Data<State>,) -> impl actix_web::Responder {

    let reader = state.servers.read().await;
    let agent = reader.get(&agent_id);

    let message = match agent {
        Some(_) => "ready",
        None => "not ready",
    };

    web::Json(AagentInfo {
        ready: agent.is_some(),
        message,
        // uptime: state.start_time.elapsed().as_secs(),
        authenticated: agent.is_some(),
    })
}