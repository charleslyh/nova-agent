## ADDED Requirements

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
