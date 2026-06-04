use crate::state::{AppState, SseEvent};
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    response::IntoResponse,
};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub async fn sse_handler(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        match msg {
            Ok(SseEvent { kind, payload }) => {
                let data = payload.to_string();
                Some(Ok::<Event, Infallible>(
                    Event::default().event(kind).data(data),
                ))
            }
            Err(_) => None, // lagged receiver — skip
        }
    });

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}
