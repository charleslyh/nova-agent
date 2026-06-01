use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use moray_sonda::Sonda;

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/events",    get(sonda_get_events))
}

async fn sonda_get_events(State(sonda): State<Arc<Sonda>>) -> axum::response::Response {
    let mut events = sonda.snapshot.subscribe();

    let event_stream = async_stream::stream! {
        while let Some(record) = events.recv().await {
            yield Ok::<Event, Infallible>(
                Event::default()
                    .id(record.revision().to_string())
                    .event("sonda")
                    .json_data(record)
                    .unwrap_or_else(|_| Event::default().event("sonda")),
            );
        }
    };
    Sse::new(event_stream).into_response()
}
