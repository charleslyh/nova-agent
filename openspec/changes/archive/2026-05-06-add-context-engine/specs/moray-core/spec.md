## ADDED Requirements

### Requirement: Context engine lifecycle contract

`moray-core` SHALL define a `ContextEngine` trait (async, object-safe, `Send + Sync`) for context lifecycle management and outbound LLM context assembly.

The trait SHALL include lifecycle methods:

- `bootstrap`
- `assemble`
- `ingest`
- `bootstrap` MUST run once before the first `assemble` in a run
- `assemble` MUST return outbound request `messages: Vec<ChatCompletionRequestMessage>`
- `ingest` MUST accept runtime outputs needed for subsequent assembly

`ContextEngine` implementations MUST NOT import `ToolCallAuthPolicy` and MUST NOT manage authorization state.

#### Scenario: bootstrap runs once per run

- **WHEN** a new `Agent::run` starts
- **THEN** runtime calls `ContextEngine::bootstrap` at most once before first `assemble`

#### Scenario: assemble runs before each model request

- **WHEN** runtime is about to call `ChatCompletion::completion`
- **THEN** runtime MUST call `ContextEngine::assemble` first and use returned `messages` as outbound request input

### Requirement: Session store remains persistence authority

`SessionStore` SHALL remain the persistence abstraction used by `Session` for event history.

`SessionStore` responsibilities MUST include:

- loading persisted `SessionEvent` history for replay/resume/sequence sync,
- appending session/agent events (including streaming text delta/tool-call related events represented in persisted `SessionEvent`),
- clearing persisted state on reset.

#### Scenario: session persists user and agent events

- **WHEN** `Session` executes invoke/resume/reset flows
- **THEN** persisted history writes MUST go through `SessionStore`

### Requirement: Store-engine consistency boundary

`Session` and `ContextEngine` MUST maintain a consistency boundary so assembled request context reflects required committed persisted state.

#### Scenario: assemble sees latest committed state

- **WHEN** `SessionStore` commits events required for next model request
- **THEN** `ContextEngine::assemble` MUST observe those commits (via revision/version handshake or equivalent) before emitting request `messages`

### Requirement: Preamble is assembled by engine

System/preamble request context SHALL be produced by engine assembly, not by caller-supplied transcript messages.

No legacy compatibility mode is defined where replayed transcript `System` entries are treated as an alternate request-context source.

#### Scenario: preamble-capable engine injects system message

- **WHEN** `assemble` prepends a fixed `System` message
- **THEN** next model request begins with that system message

## MODIFIED Requirements

### Requirement: Dynamic polymorphism via trait objects (no static generics on Agent)

`Agent::new` SHALL continue to take trait-object completion/toolbox dependencies and construction-time options (including streaming mode).  
`Agent` MUST NOT take `ContextEngine` as a construction parameter.

#### Scenario: construction does not bind run-scoped context engine

- **WHEN** inspecting `Agent::new` signature
- **THEN** it MUST NOT include a `ContextEngine` parameter

### Requirement: Session runtime orchestration in moray-core module

`Session` remains orchestration owner and SHALL:

- append/load/clear persisted event history through `SessionStore`,
- obtain or bind a context engine for each run,
- call `agent.run(context_engine, cancellation)`,
- continue managing toolbox authorization flow as before.

#### Scenario: session wires store and engine for run

- **WHEN** a new invoke/resume run starts
- **THEN** `Session` MUST persist/read through `SessionStore` and pass `ContextEngine` to `Agent::run`

### Requirement: Session harness factory for per-session construction

`SessionHarnessFactory` MUST provide toolbox/completion construction as before and MUST provide a way to wire `ContextEngine` creation/binding for session runs.

Factory API shape MAY be:

- direct `create_context_engine()`,
- or equivalent split between session-level provider and run-level creation.

`SessionHarnessFactory` MUST NOT expose `ToolCallAuthPolicy`.

#### Scenario: harness can wire context engine

- **WHEN** session integration wiring is assembled
- **THEN** it can supply/bind a `ContextEngine` for subsequent `Agent::run` calls

#### Scenario: harness owns store binding for context creation

- **WHEN** creating a session harness instance
- **THEN** store dependency MAY be bound at harness construction and reused by `create_context_engine()`

### Requirement: Agent run input and output align with chat completion messages and chunks

`Agent::run` SHALL:

- accept `Arc<dyn ContextEngine>`,
- accept `CancellationToken`,
- NOT accept caller-supplied transcript `messages`,
- NOT accept session identifiers or session-store handles.

Effective model request messages are sourced from `ContextEngine::assemble`.

Streaming mode is configured at `Agent::new` (or equivalent construction options), not on `run` input and not in context-engine lifecycle inputs.

Agent output remains the same outward event stream shape (`ChatResponse` / `ToolCall` / `Finished` baseline).

#### Scenario: run does not accept transcript messages

- **WHEN** inspecting `Agent::run` signature
- **THEN** it MUST NOT take `&[ChatCompletionRequestMessage]` (or equivalent caller-supplied transcript list)

#### Scenario: run accepts context engine handle

- **WHEN** inspecting `Agent::run` signature
- **THEN** it MUST include an `Arc<dyn ContextEngine>` parameter

#### Scenario: streaming is construction-time

- **WHEN** inspecting `Agent::new` and `Agent::run` signatures
- **THEN** streaming mode MUST be configured at construction-time and MUST NOT appear in context-engine lifecycle input payloads
