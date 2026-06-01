# moray-channels Specification

## Purpose

Defines **`moray-channels`**: shared IM channel framework (`ImChannel` and related types), **catalog persistence** (`channels.toml`), and **connector supervision** (`ChannelsManager`). Platform protocol implementations (QQ, WeCom) live in `moray-extensions`; applications inject `ChannelFactoryFn` at bootstrap.

`moray-channels` MUST NOT depend on `moray-sonda` or `moray-extensions`.

## Requirements

### Requirement: Channel catalog

`moray-channels` SHALL provide `ChannelCatalog` backed by a caller-supplied `channels.toml` path.

#### Scenario: Channel catalog primary keys

- **WHEN** a caller opens a channel catalog file
- **THEN** entries MUST use `channel_id` as the primary key
- **AND** each entry MUST bind exactly one `session_id` (1:1)
- **AND** entries MUST store `channel_type` and opaque `data` (JSON)

#### Scenario: Secret merge and redact via registered ops

- **WHEN** an application opens `ChannelCatalog` with per-type `ChannelCatalogOps` (typically from `moray-extensions`)
- **AND** catalog `upsert` receives platform `data` for a registered channel type
- **THEN** the registered `merge` hook MUST merge secrets with existing on-disk values when the client resubmits redacted placeholders
- **AND** `redact_data` MUST invoke the registered `redact` hook to mask secrets for HTTP responses
- **AND** unregistered channel types MUST pass through `data` unchanged

### Requirement: ChannelsManager supervision

`moray-channels` SHALL provide `ChannelsManager` with `start`, `stop`, `restart`, and `start_all`.

#### Scenario: Per-channel lifecycle

- **WHEN** a channel is started, restarted, or stopped
- **THEN** only the affected `channel_id` connector MUST be touched
- **AND** other running connectors MUST remain connected

#### Scenario: Outbound via SessionLiveEvents

- **WHEN** a channel connector starts
- **THEN** the supervisor MUST subscribe to live session events for the bound `session_id` via `SessionLiveEvents`
- **AND** MUST forward `AgentResponse`, `TurnFinish`, and `Reset` events to `ImChannel::on_session_event`

#### Scenario: Inbound dispatch

- **WHEN** an `ImChannel` emits `InboundMessage::User`
- **THEN** the supervisor MUST call `LiveSessions::submit` (or `reset` for `/new`) on the bound `session_id`
- **WHEN** an `ImChannel` emits `InboundMessage::Auth`
- **THEN** the supervisor MUST route the decision through the injected `ToolCallAuthorizer`

### Requirement: SessionLiveEvents abstraction

`moray-channels` SHALL define `SessionLiveEvents` for subscribing to live `moray_session::SessionEvent` values without depending on a concrete transcript store implementation.

#### Scenario: Application provides live events

- **WHEN** an application constructs `ChannelsManager`
- **THEN** it MUST supply `Arc<dyn SessionLiveEvents>` (for example an adapter over `SondaSessionTranscripts` in `moray-sonda`)

### Requirement: IM channel framework

`moray-channels` SHALL provide the shared IM connector surface used by supervisors and platform crates:

- `ImChannel` trait and associated types (`InboundMessage`, `ChannelRun`, `ApprovalDecision`, …)
- `ChannelError` (catalog, connector, and live-events errors in `error.rs`)
- `ChannelCatalogOps` with `ChannelDataMergeFn` / `ChannelDataRedactFn` for per-type catalog hooks at bootstrap

Outbound `render` helpers live in `moray-extensions` (`channels/render`) for QQ/WeCom implementations.
TLS crypto provider initialization is an application-level contract and MUST be performed once at top-level bootstrap before any rustls-based handshake.

#### Scenario: Platform crate implements trait

- **WHEN** `moray-extensions` registers a QQ or WeCom factory
- **THEN** the factory MUST return `Arc<dyn moray_channels::ImChannel>`
- **AND** MUST NOT require `moray-channels` to depend on `moray-extensions`

### Requirement: No platform protocol code in moray-channels

`moray-channels` MUST NOT implement QQ or WeCom protocol logic. Application `ChannelFactoryFn` closures (typically wired from `moray-extensions`) construct `Arc<dyn moray_channels::ImChannel>` instances.
