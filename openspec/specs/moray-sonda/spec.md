# moray-sonda Specification

## Purpose

Defines **`moray-sonda`**: the **Sonda application framework** — configuration stores, session admission, in-process runtime registry wiring, and shared app errors for building desktop-class agents. **Sonda does not choose** which tools, preamble text, or env-based completion strategy to use; concrete applications supply those via `moray_session::Harness` (or factories) when assembling `AppState`.

## Requirements

### Requirement: Sonda package layout

The repository SHALL provide `moray-sonda` at `crates/sonda` depending on `moray-core`, `moray-session`, `moray-extensions`, and `moray-channels`.

#### Scenario: Sonda is not the HTTP server

- **WHEN** inspecting `moray-sonda`
- **THEN** it MUST NOT depend on `axum` or spawn HTTP listeners
- **AND** `moray-desktop-server` MUST own HTTP routes and bootstrap

### Requirement: Sonda TOML configuration framework

`moray-sonda` SHALL load and validate shared configuration from caller-provided filesystem paths (no fixed filenames inside the crate):

- settings file — `[[completions]]`, `[[agents]]`, turn-time `api_key` resolution
- session catalog file — default agent and per-session overrides
- channel catalog file — loaded via `moray-channels::ChannelCatalog` (see `moray-channels` spec)

#### Scenario: Settings expose resolution, not tool policy

- **WHEN** a harness needs a model endpoint for an agent
- **THEN** it MUST obtain `Endpoint` (or equivalent) via `SondaSettingsStore` indirection
- **AND** `moray-sonda` MUST NOT hard-require a single global tool list inside the framework crate as the only supported policy

### Requirement: Sonda app state framework

`moray-sonda` SHALL provide `Sonda` and `SondaBuilder` with in-process live session registry (`submit`, `reset`), catalog admission, and `SessionTranscripts` integration from `moray-extensions`.

`moray-sonda` SHALL provide `SkillCenter` (catalog / `detail` / `skills(SkillFilterKind)` / `register_from_dir` / `unregister`) mirroring the toolbox factory pattern; `SkillFilterKind` MUST support `All` and `Ids`. `register_from_dir` and `unregister` update the in-process catalog only (no filesystem delete). `Sonda::install_skill` and `Sonda::uninstall_skill` SHALL orchestrate `SkillHub` download or disk removal with catalog changes. Live sessions MUST inject registered skills into the system prompt via `SkillsSection` on `TemplatedPreambler`.

Harness instances used when activating a live session MUST be **supplied by the concrete application** (for example `SondaSessionHarness` in `moray-sonda` for desktop), not baked in as the only supported policy inside a lower crate.

#### Scenario: Desktop owns harness policy

- **WHEN** the Tauri/desktop stack starts sessions
- **THEN** the application layer MUST provide the `Harness` implementation that selects tools and preamble
- **AND** `moray-sonda` MUST only require the `Harness` + shared `Arc` dependencies needed for framework wiring

### Requirement: Sonda error surface

`moray-sonda` SHALL aggregate domain errors for settings, session catalog, and channel catalog (`SondaError`) for HTTP and bootstrap boundaries.

#### Scenario: Transcript errors map through sonda app error

- **WHEN** `Sonda` encounters `SessionTranscriptsError`
- **THEN** it MUST map into `SondaError` at the sonda framework layer

### Requirement: Sonda excludes application-constrained harnesses

`moray-sonda` MUST NOT define env-var `Harness` types or REPL-only startup helpers intended solely for one example binary; those belong in `desktop/` or other app crates.

### Requirement: IM channel orchestration

`moray-sonda` SHALL orchestrate IM channels using `moray-channels` (`ChannelCatalog`, `ChannelsManager`) and MUST re-export those types for application convenience.

`moray-sonda` SHALL provide `SessionLiveEvents` for `SondaSessionTranscripts` so `ChannelsManager` can subscribe to live session events without depending on sonda transcript types.

Platform protocol implementations MUST live in `moray-extensions` (`ImChannel` trait, QQ/WeCom connectors).

#### Scenario: Channel creation allocates session

- **WHEN** `Sonda::create_channel` is called
- **THEN** it MUST allocate distinct `channel_id` and `session_id` values
- **AND** MUST initialize the session catalog, transcript store, and in-process state for `session_id`
- **AND** MUST persist the channel catalog entry and start the connector for `channel_id`

#### Scenario: Desktop owns channel HTTP and factory registration

- **WHEN** the desktop server exposes channel CRUD
- **THEN** HTTP routes MUST live in `moray-desktop-server`
- **AND** bootstrap MUST register platform factory closures (QQ, WeCom) into `ChannelsManager` via `SondaBuilder`
- **AND** `SondaBuilder` MUST forward factories to `moray-channels::ChannelsManager` without hard-coding desktop-only types
