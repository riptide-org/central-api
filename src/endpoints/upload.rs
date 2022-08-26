use actix_web::{
    error::PayloadError,
    post,
    web::{Bytes, Data, Path, Payload},
    HttpRequest, HttpResponse,
};
use futures::StreamExt;
use log::error;

use crate::{ServerId, State};

/// Internal upload handler function for a waiting client
// Dev note: This code is split off in a separate handler function to allow testing
// via implementations/mocking.
async fn __upload<T>(mut payload: T, state: Data<State>, upload_id: ServerId) -> HttpResponse
where
    T: futures::Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    //Get uploadee channel
    let mut sender_store = state.requests.write().await;
    let sender = sender_store.remove(&upload_id).unwrap();

    //XXX timeout?
    while let Some(chk) = payload.next().await {
        if let Err(e) = sender.send(chk).await {
            error!("problem sending payload {:?}", e);
            return HttpResponse::InternalServerError().body("upload failed");
        };
    }

    HttpResponse::Ok().body("successfully uploaded")
}

/// Upload a file or metadata to a waiting client
#[post("/upload/{upload_id}")]
pub async fn upload(
    _: HttpRequest,
    payload: Payload,
    state: Data<State>,
    path: Path<ServerId>,
) -> impl actix_web::Responder {
    __upload(payload, state, path.into_inner()).await
}

#[cfg(test)]
#[cfg(not(tarpaulin_include))]
mod test {
    use actix_web::{
        error::PayloadError,
        http::StatusCode,
        web::{self, Bytes},
    };
    use futures::Stream;
    use std::{
        collections::HashMap,
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::sync::RwLock;

    use super::__upload;
    use crate::State;

    struct MockStreamer<'a>(&'a [u8], bool);

    impl Stream for MockStreamer<'_> {
        type Item = Result<Bytes, PayloadError>;
        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.1 {
                true => Poll::Ready(None),
                false => {
                    self.1 = true;
                    Poll::Ready(Some(Ok(Bytes::copy_from_slice(self.0))))
                }
            }
        }
    }

    #[tokio::test]
    async fn test_upload() {
        let upload_id = 33;
        let payload_msg = b"a long set of data";

        let state = web::Data::new(State {
            unauthenticated_servers: Default::default(),
            servers: Default::default(),
            requests: RwLock::new(HashMap::new()),
            base_url: "https://localhost:8080".into(),
            start_time: std::time::Instant::now(),
        });

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        state.requests.write().await.insert(upload_id, tx);

        let payload = MockStreamer(payload_msg, false);

        let resp = __upload(payload, state, upload_id).await;

        assert_eq!(resp.status(), StatusCode::OK);

        assert_eq!(
            rx.try_recv().unwrap().unwrap(),
            Bytes::copy_from_slice(payload_msg)
        );
    }
}
