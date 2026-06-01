mod channels;
mod events;
mod sessions;
mod settings;
mod skills;
mod tools;

use std::sync::Arc;

use axum::Router;
use moray_sonda::Sonda;
use tower_http::cors::{Any, CorsLayer};

pub(crate) fn create_router(state: Arc<Sonda>) -> Router {
    #[rustfmt::skip]
    let router = Router::new()
        .nest("/tools",     tools::router())
        .nest("/tool-auth", tools::auth_router())
        .nest("/skills",    skills::router())
        .nest("/settings",  settings::router())
        .nest("/sonda",     events::router())
        .nest("/channels",  channels::router())
        .nest("/sessions",  sessions::router());

    let cors = CorsLayer::new()
        .allow_origin(
            "http://localhost:5173"
                .parse::<axum::http::HeaderValue>()
                .expect("valid CORS origin"),
        )
        .allow_methods(Any)
        .allow_headers(Any);

    router.layer(cors).with_state(state)
}
