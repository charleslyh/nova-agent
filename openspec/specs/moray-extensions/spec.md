# moray-extensions Specification

## Purpose

Defines **`moray-extensions`**: **extension implementations** of traits from `moray-core` and `moray-session`, plus small **domain-generic** helpers that compose those implementations. Extensions MUST NOT encode a specific product's TOML layout, env-var CLI conventions, or a full `Harness` strategy for one application.

## Requirements

### Requirement: Extensions package layout

The repository SHALL provide `moray-extensions` at `crates/extensions` depending on `moray-core` and `moray-session`.

#### Scenario: Extensions exclude application frameworks

- **WHEN** inspecting `moray-extensions` public modules
- **THEN** it MUST NOT expose `server.toml` / `sessions.toml` loaders, `AppState`, `AppStateBuilder`, or HTTP types

### Requirement: Extensions transcript store

`moray-extensions` SHALL provide JSONL-backed `SessionTranscripts` under `moray_extensions::transcripts`, implementing `moray_session::SessionEventSink`, with `subscribe(from_seq)`, `load`, and `replay_session_records`.

#### Scenario: Transcripts depend on session event types

- **WHEN** consuming `SessionTranscripts`
- **THEN** row payloads MUST use `moray_session::SessionEvent`
- **AND** replay MUST live alongside persistence under `moray_extensions::transcripts`

### Requirement: Extensions trait implementations

`moray-extensions` SHALL provide default implementations for domain traits, including at minimum:

- `moray_core::ChatCompletion` (OpenAI-compatible adapter)
- `moray_extensions::preambles::TemplatedPreambler` with template substitutions and optional `PreambleSection` append (e.g. `SkillsSection`)
- `moray_extensions::skills` — load `SKILL.md` / `SKILL.toml`, render skills into system prompt XML
- `moray_core::Tool` (built-in tool types)
- `moray_core::ContextEngine` (`CompositeContextEngine` in `context/composite.rs`)
- `moray_core::ToolCallInterceptor` policies (e.g. always-ask, which also implements `moray_channels::ToolCallReplyRouter` for approval replies)

#### Scenario: TemplatedPreambler injects system prompt on assemble

- **WHEN** `CompositeContextEngine` is constructed with a `TemplatedPreambler` pipeline node
- **THEN** `assemble` MUST prepend or replace a leading `System` message from rendered template substitutions
- **AND** registered `PreambleSection` implementations (such as `SkillsSection`) MUST be appended after template substitution when non-empty

#### Scenario: SkillsSection renders authorization and available skills

- **WHEN** a non-empty skill list is passed to `SkillsSection`
- **THEN** the rendered fragment MUST include skills authorization guidance and an `<available_skills>` XML block (compact by default; full instructions when `always` or full mode)

### Requirement: Extensions generic session helpers

`moray-extensions` MAY provide composition helpers that are not themselves trait implementations but are reusable across applications, such as hydrating `CompositeContextEngine` from `SessionTranscripts` via `replay_session_records`.

#### Scenario: memory_context_from_transcript is extensions

- **WHEN** any app reloads context from an on-disk transcript
- **THEN** it MAY call a helper exposed from `moray-extensions`

### Requirement: Extensions exclude application-constrained Harness

Implementations of `moray_session::Harness` that encode **one application's configuration contract** (for example reading `MORAY_OPENAI_*` environment variables for a CLI REPL) MUST NOT live in `moray-extensions`.

#### Scenario: EnvHarness is not in extensions

- **WHEN** inspecting `moray-extensions`
- **THEN** it MUST NOT define `EnvHarness` or `MORAY_OPENAI_*` env resolution as the primary configuration path

### Requirement: Extensions exclude REPL-only bootstrap recipes

Single-entry REPL helpers that encode one application's startup sequence (for example `open_session_runtime` tied to a fixed session id workflow) MUST live in that application crate, not `moray-extensions`, unless promoted later as a documented generic pattern.
