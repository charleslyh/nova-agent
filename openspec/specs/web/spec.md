# web Specification

## Purpose
TBD - created by archiving change add-tauri-vue-single-session-chat-app. Update Purpose after archive.
## Requirements
### Requirement: Web app lives under desktop
The repository SHALL provide the Vue web app as a sub-app under the top-level `desktop/` directory at `desktop/web`, colocated with the chat server and desktop client sub-apps.

#### Scenario: Web app is colocated with other desktop sub-apps
- **WHEN** reviewing the repository layout after implementation
- **THEN** the web app's source, package files, and configuration MUST live at `desktop/web`
- **AND** the web app MUST NOT be hosted inside reusable Moray runtime crates

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
- **AND** it MUST register a listener for the SSE event name `session`
- **AND** it MUST rely on the browser's automatic reconnect with `Last-Event-ID` for resume

### Requirement: Stream-driven conversation rendering
The web app SHALL render conversation progress from `ChatClient`-delivered session events without distinguishing replay from live at the transport layer.

#### Scenario: Session events update the transcript
- **WHEN** the `ChatClient` delivers an `event: session` item
- **THEN** the UI MUST update the transcript and turn status from that event
- **AND** it MUST handle tool authorization and other interactive behavior through the same event handler when applicable

#### Scenario: Web app does not embed conversation runtime logic
- **WHEN** reviewing the web app's source code
- **THEN** Vue code MUST NOT instantiate or reimplement Moray agent / session execution logic locally
- **AND** all conversation behavior MUST be delegated through the `ChatClient` abstraction

### Requirement: Tool authorization prompt handling in the web app
The web app SHALL surface tool authorization requests delivered through `ChatClient` session events and allow the user to approve or deny them.

#### Scenario: Authorization request is displayed from session events
- **WHEN** the `ChatClient` delivers a tool permission request inside an `event: session` item
- **THEN** the web app MUST display a prompt that identifies the pending tool call using information available from the event stream
- **AND** the prompt MUST allow the user to approve or deny the request

#### Scenario: Authorization reply is sent through the ChatClient
- **WHEN** the user approves or denies a pending tool authorization prompt
- **THEN** the web app MUST forward the decision through the `ChatClient` abstraction along with the matching tool call id
- **AND** subsequent session stream events MUST continue to update the UI

### Requirement: Web app exposes agents settings from the title bar
The web app SHALL add an agents settings entry button to the title bar. Activating the button SHALL open an agents configuration page or dialog without navigating away from the active chat session.

#### Scenario: Title bar opens agents settings
- **WHEN** the user clicks the agents settings button in the title bar
- **THEN** the web app MUST display the agents settings surface
- **AND** the active chat transcript and composer state MUST remain available when the settings surface is closed

#### Scenario: Settings data loads through ChatClient
- **WHEN** the agents settings surface opens
- **THEN** the web app MUST load agents and completion options through the `ChatClient` abstraction
- **AND** Vue components MUST NOT call `fetch`, `EventSource`, Tauri commands, or other transport-specific APIs directly for this behavior

### Requirement: Agents settings displays agent cards
The agents settings surface SHALL display configured agents as cards. Each card SHALL show enough information for users to distinguish the agent, including its display name and selected completion display name when available.

#### Scenario: Agent cards are rendered from server data
- **WHEN** the server returns multiple configured agents
- **THEN** the settings surface MUST render one card for each agent
- **AND** each card MUST be keyed by the stable agent id rather than by display name

#### Scenario: Current session agent is visually identifiable
- **WHEN** the settings surface renders an agent that is currently bound to the active session
- **THEN** that agent card MUST visually indicate that it is the current session agent

### Requirement: Agent card opens detail configuration card
Clicking an agent card SHALL open a detail configuration card for that agent. In this change, the detail configuration SHALL include a completion dropdown menu and SHALL NOT include completion credential editing fields.

#### Scenario: Detail card shows completion dropdown
- **WHEN** the user opens an agent detail card
- **THEN** the card MUST display a dropdown containing the configured completion options
- **AND** the dropdown's selected value MUST match the agent's current `completion_id`

#### Scenario: Completion selection updates the agent
- **WHEN** the user selects a different completion in the agent detail dropdown
- **THEN** the web app MUST save the agent's updated `completion_id` through `ChatClient`
- **AND** the visible agent card MUST reflect the updated completion after the save succeeds

#### Scenario: Completion credentials are not editable in web
- **WHEN** the user opens the agent detail card
- **THEN** the UI MUST NOT expose fields for editing API key, base URL, model, or completion capabilities
- **AND** the UI MUST only allow choosing among completion records already known to the server

### Requirement: Composer allows switching the current session agent
The chat composer SHALL include an Agent selection menu in its lower-left action area. Changing the selection SHALL update the active session's persisted agent binding through `ChatClient`.

#### Scenario: Composer displays current session agent
- **WHEN** the chat UI initializes
- **THEN** the composer Agent menu MUST show the agent currently bound to the active session
- **AND** if the session has no previous binding, it MUST show the server's default agent after initialization

#### Scenario: Composer switch persists binding
- **WHEN** the user selects a different agent from the composer Agent menu
- **THEN** the web app MUST request a session-agent switch through `ChatClient`
- **AND** subsequent messages in that session MUST be sent after the server has accepted the new binding

#### Scenario: Composer switch during active turn is saved for next turn
- **WHEN** the active session status is running
- **THEN** the composer Agent menu MAY remain available
- **AND** if the user switches agents, the new binding MUST apply from the next post onward for that session (same semantics as when no turn is running); additional in-UI explanatory copy is optional for this change

### Requirement: ChatClient includes agent management operations
The web app SHALL extend the `ChatClient` abstraction with methods for listing agent settings data, reading a session's current agent, switching a session's agent, and updating an agent's completion selection. Session-scoped methods SHALL accept a `session_id` argument (or equivalent) so the transport matches the server's multi-session-capable HTTP shape; the current desktop web client MAY pass only `default`.

#### Scenario: Components use ChatClient for agent operations
- **WHEN** reviewing the web app source code after implementation
- **THEN** components responsible for agents settings or composer agent selection MUST call `ChatClient` methods for all agent operations
- **AND** only the HTTP-backed `ChatClient` implementation may know the concrete HTTP routes

#### Scenario: Existing chat operations remain available
- **WHEN** the `ChatClient` abstraction is extended for agent operations
- **THEN** existing post message, subscribe events, reply tool authorization, and reset behavior MUST continue to be exposed through the same abstraction

