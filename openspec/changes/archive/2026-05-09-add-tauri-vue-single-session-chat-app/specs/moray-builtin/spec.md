## ADDED Requirements

### Requirement: Moray builtin package
The repository SHALL provide a new top-level Rust package `moray-builtin` containing reusable, ready-to-use building blocks (stores, tools) that depend on `moray-core` / `moray-sessions` and are usable by both `demo/` and `desktop/server`.

#### Scenario: Builtin lives at the workspace root
- **WHEN** reviewing the repository layout after implementation
- **THEN** the package MUST live at `builtin/` at the repository root
- **AND** its Cargo crate name MUST be `moray-builtin`

#### Scenario: Builtin is a member of the root workspace
- **WHEN** inspecting the root `Cargo.toml`
- **THEN** `builtin` MUST appear as a member of the workspace
- **AND** `moray-builtin` MUST be buildable via `cargo build -p moray-builtin`

### Requirement: Builtin JSONL transcript store
The `moray-builtin` package SHALL provide a JSONL-backed implementation of `moray_sessions::SessionStore` (with subscribe and load capabilities) named `JsonlTranscriptStore`, exposed under the `stores` module.

#### Scenario: JsonlTranscriptStore is exposed by builtin
- **WHEN** consuming `moray-builtin`
- **THEN** the type `moray_builtin::stores::JsonlTranscriptStore` MUST be accessible
- **AND** it MUST implement `moray_sessions::SessionStore`

#### Scenario: Behavior matches the prior demo implementation
- **WHEN** comparing observable behavior of `moray_builtin::stores::JsonlTranscriptStore` to the previous demo-local implementation
- **THEN** persistence format, load semantics, subscribe semantics (including starting sequence and fan-out behavior), and clear semantics MUST be unchanged

### Requirement: Builtin calc tool
The `moray-builtin` package SHALL provide a `CalcTool` under the `tools` module, implementing `moray_core::Tool`, that evaluates simple four-arithmetic expressions using a double-stack algorithm with operator precedence.

#### Scenario: CalcTool is exposed by builtin
- **WHEN** consuming `moray-builtin`
- **THEN** the type `moray_builtin::tools::CalcTool` MUST be accessible
- **AND** it MUST implement `moray_core::Tool`

#### Scenario: CalcTool input and output JSON shape
- **WHEN** invoking `CalcTool` with arguments
- **THEN** the input MUST be a JSON object of shape `{ "expression": String }`
- **AND** the success output MUST be a JSON object of shape `{ "value": Number }`
- **AND** the failure output MUST be a JSON object of shape `{ "error": String }` (e.g. division by zero, parse failure)

#### Scenario: CalcTool supports operator precedence
- **WHEN** `CalcTool` evaluates an expression with mixed `+ - * /`
- **THEN** evaluation MUST respect standard arithmetic operator precedence (multiplicative before additive)

#### Scenario: CalcTool surfaces user authorization on every call
- **WHEN** any caller invokes `CalcTool` through a `Toolbox` whose `ToolCallAuthorizer` is the one shipped alongside `CalcTool`
- **THEN** the policy MUST return `AuthDecision::AskUser` for every invocation
- **AND** the toolbox MUST surface a permission request through the standard `moray-sessions` event flow before executing the tool
