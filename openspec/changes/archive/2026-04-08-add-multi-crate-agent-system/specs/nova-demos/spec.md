> Historical note (2026-04-29): this archived spec uses old names (`OpenAICompletionProvider`, `nova-core` `openai` feature). Current implementation provides OpenAI adapter from `nova-builtin` (`nova_builtin::completions::OpenAIChatCompletion`).

## ADDED Requirements

### Requirement: Non-publishable demo package

The demo workspace package SHALL declare **`publish = false`** in its manifest so it is not published to crates.io; its name MAY be short and repository-local (for example `nova-demo`).

#### Scenario: Crate is not publishable by manifest

- **WHEN** inspecting the demo crate `Cargo.toml`
- **THEN** `publish` MUST be `false` or equivalent Cargo configuration that prevents accidental publishing

### Requirement: chat example

The demo crate SHALL provide a **`chat`** example that loads a transcript, splits QA sections, prints archived completed sections for replay, initializes the agent with a **`CompletionProvider`** implementation suitable for interactive use, resumes any pending section via `resume`, then enters a REPL that calls `invoke` for new user queries.

The example SHALL use **`nova_core::OpenAICompletionProvider`** (requires **`nova-core`** dependency with feature **`openai`**) for **`CompletionProvider`**, constructed via **`nova_demo::completion_provider_from_env`**, which reads **`NOVA_OPENAI_API_KEY`**, **`NOVA_OPENAI_BASE_URL`**, and **`NOVA_OPENAI_MODEL`**, and SHALL build **`CompletionCapabilities`** from the environment (including optional **`NOVA_OPENAI_PREFILL_SUPPORTED`**) as documented on **`openai_env`**. A demo-level **mock toolbox** (for example echo) MAY be used to illustrate tool listing, authorization, and tool results without requiring a separate tools backend.

In this phase, transcript loading and QA segmentation MAY live in the demo crate or in minimal helpers in `nova-core` that do not constitute the future `nova-session` product API.

#### Scenario: Example documents required environment variables

- **WHEN** reading the `chat` example module documentation (or **`nova_demo::openai_env`**)
- **THEN** it MUST list **`NOVA_OPENAI_API_KEY`**, **`NOVA_OPENAI_BASE_URL`**, and **`NOVA_OPENAI_MODEL`** as required for the live chat client, and document optional capability-related variables (for example **`NOVA_OPENAI_PREFILL_SUPPORTED`**) as applicable

#### Scenario: Workspace tests stay offline

- **WHEN** CI runs `cargo test` for the workspace
- **THEN** tests MUST pass without network access or live model APIs (mock **`CompletionProvider`** / toolbox in **`nova-demo`** or test-only doubles)

### Requirement: Shared stream consumption for resume and invoke

The `chat` example SHALL delegate processing of streaming agent events from both `resume` and `invoke` to a single shared implementation (for example one **turn runner** function) that handles text output and tool authorization prompts consistently.

Archived transcript replay for completed sections SHOULD use the **same** user-visible patterns as interactive turns (no special `[replay:]` prefixes or extra section banners); pending resume SHOULD use the same turn runner as new `invoke` calls.

#### Scenario: No duplicated match loop in main

- **WHEN** reviewing the example source for pending-section handling versus the REPL loop
- **THEN** the event matching and side-effect dispatch MUST be centralized in one reusable unit

### Requirement: Integration tests with mock chat completion and toolbox

The workspace SHALL include integration tests (in **`nova-core`** with dev-dependency on **`nova-demo`** and/or in the demo crate as appropriate) that verify core behaviors using mock **`CompletionProvider`** and **`Toolbox`**, including recovery after authorization suspension and the non-prefill partial-turn discard policy.

#### Scenario: Tests pass without external services

- **WHEN** CI runs `cargo test` for the affected crates
- **THEN** these integration tests MUST pass without network access
