#![forbid(unsafe_code)]
//! **Moray channels** — IM channel framework, catalog, and connector management.

pub mod catalog;
pub mod channel;
pub mod error;
#[macro_use]
mod i18n;
pub mod manager;

pub use catalog::{
    ChannelCatalog, ChannelCatalogOps, ChannelDataMergeFn, ChannelDataRedactFn,
};
pub use channel::{
    ApprovalDecision, AuthReply, ChannelRun, ImChannel, InboundMessage, UserMessage,
};
pub use error::{
    ChannelCatalogError, ChannelError, FileIoError, InvalidContent, MissingReference, Result,
    SessionLiveEventsError,
};
pub use manager::{
    ChannelEntry, ChannelFactoryFn, ChannelsManager, SessionLiveEvents, ToolCallReplyRouter,
};
