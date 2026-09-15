# Project Context

## Purpose

Nova is a multi-crate Rust workspace providing the **agent core**: an explicit
**ReAct state machine** plus append-only **domain events**, with the
**`ChatCompletion`** abstraction in **`nova-core`** and concrete provider
adapters plus built-in tools in **`nova-extensions`**.

## Tech Stack

- **Language**: Rust, edition **2021**
- **Async**: **Tokio** (workspace standard)
- **Layout**: **`crates/core`** → **`nova-core`**; **`crates/extensions`** → **`nova-extensions`**.
- **`nova-core`**: **`completion`** (`ChatCompletion`, `ChatCompletionRequestMessage`, `ChatCompletionResponseChunk`, `ChatCompletionCapabilities`), **`toolbox`**, **`agent`**, **`context`**, and shared types/errors
- **`nova-extensions`**: trait implementations and built-in tools (completions, context, preambles, skills, tools)

## Project Conventions

### Code Style

- `rustfmt` defaults; `cargo clippy -- -D warnings` recommended in CI

### Architecture Patterns

- **DIP**: **`nova-core`** depends on traits, not HTTP SDKs; provider adapters live in separate crates
- **Dynamic dispatch**: **`Agent::new`** takes **`Arc<dyn ChatCompletion + Send + Sync>`** and **`Arc<dyn Toolbox + Send + Sync>`**—runtime-selected models and toolboxes without **`Agent<M, T>`** generics
- **Messages**: **`ChatCompletionRequestMessage`** by role — `User` / `Assistant` / `Tool` (**`call_id`** on tool results). **`ToolManifest.parameters`** is a JSON Schema string

### Testing Strategy

- **Unit tests**: agent replay and toolbox behavior in **`nova-core`**
- **Integration tests**: workspace `cargo test` (offline mocks; no live model APIs)

### Git Workflow

- Conventional commits (repository-level rule); OpenSpec proposals under `openspec/changes/`

## Domain Context

## Important Constraints

## External Dependencies

- **`async-openai`**: used by **`nova-extensions`** for OpenAI-compatible HTTP streaming
