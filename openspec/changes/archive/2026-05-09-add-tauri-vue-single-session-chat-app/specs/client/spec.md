## ADDED Requirements

### Requirement: Client lives under desktop
The repository SHALL provide the Tauri v2 desktop client as a sub-app under the top-level `desktop/` directory at `desktop/client`, colocated with the chat server and web app sub-apps.

#### Scenario: Client is colocated with other desktop sub-apps
- **WHEN** reviewing the repository layout after implementation
- **THEN** the Tauri desktop client's source and configuration MUST live at `desktop/client`
- **AND** the client MUST NOT be hosted inside reusable Nova runtime crates

### Requirement: Desktop-only Tauri v2 build
The Tauri desktop client SHALL target Tauri v2 with desktop-only build targets and SHALL NOT enable mobile platforms in this change.

#### Scenario: Tauri version and platform scope
- **WHEN** inspecting the Tauri configuration
- **THEN** the project MUST be configured against Tauri v2
- **AND** mobile platform targets MUST be disabled

### Requirement: Desktop window hosts the web app
The Tauri desktop client SHALL open a desktop window and host `desktop/web` as the window's web content.

#### Scenario: App launches a desktop chat window
- **WHEN** the desktop app is run in local development mode
- **THEN** the desktop client MUST open a window containing the web app
- **AND** the window MUST allow the user to interact with the chat UI

### Requirement: Client manages chat server lifecycle in-process
The desktop client SHALL embed `desktop/server` as a Tokio task in its own Tauri process, starting it on app startup and shutting it down gracefully on app exit.

#### Scenario: Server is started in-process during Tauri setup
- **WHEN** the desktop app launches
- **THEN** the desktop client MUST call `desktop/server`'s `start(opts)` from the Tauri `setup` hook
- **AND** the resulting `ServerHandle` MUST be stored in Tauri state for later use
- **AND** the server task MUST run in the desktop client's Tokio runtime, not as a separate OS process

#### Scenario: Server lifetime is bounded by the Tauri process
- **WHEN** the desktop app launches the in-process server
- **THEN** there MUST NOT be any separate desktop-server OS process for chat behavior
- **AND** ending the desktop client's process MUST end the server

#### Scenario: Server stops gracefully when the desktop app exits
- **WHEN** the user closes the desktop app or exits the Tauri process
- **THEN** the desktop client MUST invoke `ServerHandle::shutdown()` so in-flight requests can complete
- **AND** the desktop client MUST NOT leave orphaned chat-server tasks or sockets after exit

### Requirement: Server URL is exposed via a single Tauri command
The desktop client SHALL expose the chat server's bound URL to the web app through a single Tauri command and SHALL NOT use other channels (window globals, URL injection, etc.) for this purpose.

#### Scenario: get_server_url returns the bound URL
- **WHEN** `desktop/web` invokes the Tauri command `get_server_url`
- **THEN** the desktop client MUST return a string of the form `http://<local_addr>` resolved from the live `ServerHandle`
- **AND** the URL MUST resolve to a loopback address

#### Scenario: Server URL is not injected via globals
- **WHEN** reviewing how the web app discovers the chat server's address
- **THEN** the desktop client MUST NOT inject the server URL through window globals, URL query parameters, or any channel other than the `get_server_url` Tauri command

### Requirement: Tauri commands are limited to system and shell concerns
The desktop client's Tauri command surface SHALL be limited to system, window, and lifecycle concerns and SHALL NOT expose chat-domain operations.

#### Scenario: Chat-domain operations are not Tauri commands
- **WHEN** reviewing the Tauri command list exposed by the desktop client
- **THEN** there MUST NOT be Tauri commands for posting chat messages, replying to tool authorizations, resetting the active session, or streaming session events in this change
- **AND** chat-domain operations MUST be served by the chat server instead

#### Scenario: System integration commands are allowed
- **WHEN** the desktop client exposes commands for window management, runtime info, or chat server lifecycle (such as `get_server_url`)
- **THEN** those commands MUST be limited to local system / shell responsibilities

### Requirement: Desktop is a self-contained Cargo workspace
The `desktop/` directory SHALL form its own Cargo workspace, excluded from the root workspace, and SHALL reference Nova runtime crates and `nova-builtin` via path dependencies.

#### Scenario: Root workspace excludes desktop
- **WHEN** inspecting the root `Cargo.toml`
- **THEN** it MUST contain `exclude = ["desktop"]`
- **AND** `desktop/` MUST NOT appear in the root workspace's `members`

#### Scenario: Desktop workspace owns its members
- **WHEN** inspecting `desktop/Cargo.toml`
- **THEN** it MUST declare its own `[workspace]` with members `server` and `client/src-tauri`
- **AND** dependencies on `nova-core`, `nova-sessions`, and `nova-builtin` MUST be declared via `path` references
