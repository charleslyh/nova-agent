#![forbid(unsafe_code)]
//! **Moray desktop HTTP server** — Axum routes and loopback gateway lifecycle.

mod error;
mod gateway;
mod routes;

pub use gateway::{SondaGateway, SondaGatewayError};
