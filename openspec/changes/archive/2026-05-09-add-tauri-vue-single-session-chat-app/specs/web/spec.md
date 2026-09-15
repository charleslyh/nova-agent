## ADDED Requirements

### Requirement: Web app lives under desktop
The repository SHALL provide the Vue web app as a sub-app under the top-level `desktop/` directory at `desktop/web`, colocated with the chat server and desktop client sub-apps.

#### Scenario: Web app is colocated with other desktop sub-apps
- **WHEN** reviewing the repository layout after implementation
- **THEN** the web app's source, package files, and configuration MUST live at `desktop/web`
- **AND** the web app MUST NOT be hosted inside reusable Nova runtime crates

### Requirement: Web stack is Vue 3 + Vite + pnpm in plain JavaScript
The web app SHALL be implemented as a pnpm package using Vue 3 and Vite in plain JavaScript and SHALL NOT introduce TypeScript or a UI framework in this change.

#### Scenario: Toolchain is pnpm + Vite + Vue 3 + JavaScript
- **WHEN** inspecting `desktop/web`'s package files
- **THEN** the package MUST be managed by pnpm
- **AND** it MUST use Vue 3 and Vite
- **AND** its source files MUST be plain JavaScript, not TypeScript

#### Scenario: No UI framework is introduced
- **WHEN** inspecting the web app's dependencies
- **THEN** the project MUST NOT include a UI framework dependency such as Naive UI, Element Plus, or Ant Design Vue

### Requirement: Vue single-session chat UI
The web app SHALL provide a Vue chat UI for one active session, including a transcript view, an input composer, a turn status indicator, and a reset control.

#### Scenario: User can submit a message
- **WHEN** the user types a message and submits the composer
- **THEN** the UI MUST send the message through the `ChatClient` abstraction
- **AND** the UI MUST display the resulting conversation updates

#### Scenario: Reset control returns the UI to an empty conversation
- **WHEN** the user activates the reset control
- **THEN** the UI MUST request a session reset through the `ChatClient` abstraction
- **AND** the rendered transcript MUST be cleared or replaced to match the reset session

#### Scenario: No multi-session UI is exposed
- **WHEN** using the initial web app
- **THEN** the UI MUST NOT expose controls for creating, listing, switching, or deleting multiple sessions
- **AND** all conversation actions MUST target the single active session

### Requirement: Server URL is obtained via Tauri command at startup
The web app SHALL obtain the chat server's base URL by invoking the desktop client's `get_server_url` Tauri command once at startup and SHALL NOT hardcode a URL or read it from window globals or URL parameters.

#### Scenario: Web app calls get_server_url on startup
- **WHEN** the web app initializes its `ChatClient`
- **THEN** it MUST `invoke('get_server_url')` once to resolve the chat server's base URL
- **AND** the resolved URL MUST be passed into the `ChatClient` constructor

#### Scenario: No build-time or global server URL
- **WHEN** reviewing the web app's source
- **THEN** the chat server URL MUST NOT be hardcoded at build time
- **AND** the web app MUST NOT read the URL from a window global or URL query parameter

### Requirement: Transport-agnostic ChatClient abstraction
The web app SHALL define a `ChatClient` abstraction that encapsulates all chat-service access, and UI components SHALL access chat behavior only through that abstraction.

#### Scenario: ChatClient is the only chat access path for UI components
- **WHEN** reviewing UI component source code
- **THEN** components MUST obtain chat behavior (post message, subscribe to session events, reply tool authorization, reset session) only via the `ChatClient` abstraction
- **AND** components MUST NOT call `fetch`, `EventSource`, Tauri commands, or other transport-specific APIs directly for chat behavior

#### Scenario: ChatClient implementation is replaceable without UI changes
- **WHEN** the underlying chat transport is changed (for example from HTTP to a future Tauri-command implementation)
- **THEN** only the `ChatClient` implementation module MUST need to change
- **AND** UI components MUST NOT need to be modified to support the new transport

### Requirement: HTTP-backed ChatClient implementation
The web app SHALL ship an HTTP-backed `ChatClient` implementation that uses `fetch` for command-shaped endpoints and `EventSource` for the SSE event stream against the chat server's base URL.

#### Scenario: Commands use fetch
- **WHEN** the `ChatClient` posts a message, replies to a tool authorization, or resets the session
- **THEN** it MUST issue an HTTP request via `fetch` against the resolved server base URL

#### Scenario: Subscription uses EventSource
- **WHEN** the `ChatClient` subscribes to session events
- **THEN** it MUST open `GET /events?from_seq=N` via `EventSource`
- **AND** it MUST register listeners for the SSE event names `replay`, `live-start`, and `live`
- **AND** it MUST rely on the browser's automatic reconnect with `Last-Event-ID` for resume

### Requirement: Stream-driven conversation rendering with replay vs live distinction
The web app SHALL render conversation progress from `ChatClient`-delivered session events and SHALL distinguish replay-phase events from live-phase events based on the SSE event name.

#### Scenario: Replay events render history without prompting
- **WHEN** the `ChatClient` delivers `event: replay` items
- **THEN** the UI MUST update the transcript and turn status from those events
- **AND** the UI MUST NOT surface tool authorization prompts for replay-phase events

#### Scenario: Live events drive interactive behavior
- **WHEN** the `ChatClient` delivers `event: live-start` followed by `event: live` items
- **THEN** the UI MUST treat the application as caught up after `live-start`
- **AND** subsequent live events MUST update the transcript and trigger interactive behavior including authorization prompts

#### Scenario: Web app does not embed conversation runtime logic
- **WHEN** reviewing the web app's source code
- **THEN** Vue code MUST NOT instantiate or reimplement Nova agent / session execution logic locally
- **AND** all conversation behavior MUST be delegated through the `ChatClient` abstraction

### Requirement: Tool authorization prompt handling in the web app
The web app SHALL surface tool authorization requests delivered through live `ChatClient` events and allow the user to approve or deny them.

#### Scenario: Authorization request is displayed for live events only
- **WHEN** the `ChatClient` delivers a tool permission request inside an `event: live` item
- **THEN** the web app MUST display a prompt that identifies the pending tool call using information available from the event stream
- **AND** the prompt MUST allow the user to approve or deny the request

#### Scenario: Authorization reply is sent through the ChatClient
- **WHEN** the user approves or denies a pending tool authorization prompt
- **THEN** the web app MUST forward the decision through the `ChatClient` abstraction along with the matching tool call id
- **AND** subsequent session stream events MUST continue to update the UI
