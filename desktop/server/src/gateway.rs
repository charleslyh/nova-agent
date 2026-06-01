//! Loopback HTTP gateway lifecycle: bind, serve, and graceful shutdown.

use std::net::SocketAddr;
use std::sync::Arc;

use moray_channels::ChannelCatalogError;
use moray_sonda::Sonda;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::routes::create_router;

#[derive(Debug, thiserror::Error)]
pub enum SondaGatewayError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Settings(#[from] moray_sonda::SondaError),

    #[error(transparent)]
    Session(#[from] moray_core::MorayError),
}

impl From<moray_sonda::SondaSettingsStoreError> for SondaGatewayError {
    fn from(err: moray_sonda::SondaSettingsStoreError) -> Self {
        Self::Settings(err.into())
    }
}

impl From<moray_sonda::SessionCatalogError> for SondaGatewayError {
    fn from(err: moray_sonda::SessionCatalogError) -> Self {
        Self::Settings(err.into())
    }
}

impl From<ChannelCatalogError> for SondaGatewayError {
    fn from(err: ChannelCatalogError) -> Self {
        Self::Settings(err.into())
    }
}

/// Loopback Axum gateway; references [`Sonda`] owned by the application host.
pub struct SondaGateway {
    pub local_addr: SocketAddr,
    cancel: CancellationToken,
    join: JoinHandle<()>,
}

impl SondaGateway {
    /// Binds `127.0.0.1:0`, builds the router, and starts `axum::serve` on the current runtime.
    pub async fn spawn(sonda: Arc<Sonda>) -> Result<Self, SondaGatewayError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let local_addr = listener.local_addr()?;
        let app = create_router(sonda);

        let cancel = CancellationToken::new();
        let shutdown = cancel.clone();

        let join = tokio::spawn(async move {
            let shutdown_signal = async move {
                shutdown.cancelled().await;
            };

            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal)
                .await;
        });

        Ok(Self {
            local_addr,
            cancel,
            join,
        })
    }

    /// Stops accepting HTTP requests and waits for in-flight requests to finish.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.join.await;
        info!("gateway shutdown complete");
    }
}
