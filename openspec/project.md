# Project Context

## Purpose

Moray is a multi-crate Rust workspace for a **recoverable agent runtime**: an explicit state machine plus append-only **domain events**, with **`ChatCompletion`** abstractions in **`moray-core`** and concrete provider adapters in reusable packages such as **`moray-sonda`**.

## Tech Stack

- **Language**: Rust, edition **2021**
- **Async**: **Tokio** (workspace standard)
- **Layout**: directories **`core/`** → package **`moray-core`**; **`crates/session`**, **`crates/extensions`**, **`crates/sonda`**, **`desktop/*`** for application and integration layers.
- **`moray-core` (high level)**: **`completion`** (`ChatCompletion`, `ChatCompletionRequestMessage`, `ChatCompletionResponseChunk`, `ChatCompletionCapabilities`), **`toolbox`**, **`agent`**, **`context`**, and shared types/errors
- **`moray-sonda`**: batteries-included runtime building blocks, including provider adapters (currently **`OpenAIChatCompletion`** via [`async-openai`](https://crates.io/crates/async-openai))

## Project Conventions

### Code Style

- `rustfmt` defaults; `cargo clippy -- -D warnings` recommended in CI

### Architecture Patterns

- **DIP**: **`moray-core`** depends on traits, not HTTP SDKs; provider adapters live in separate crates
- **Dynamic dispatch**: **`Agent::new`** takes **`Arc<dyn ChatCompletion + Send + Sync>`** and **`Arc<dyn Toolbox + Send + Sync>`**—runtime-selected models and toolboxes without **`Agent<M, T>`** generics
- **Messages**: **`ChatCompletionRequestMessage`** by role — `User` / `Assistant` / `Tool` (**`call_id`** on tool results). **`ToolManifest.parameters`** is a JSON Schema string
- **Recovery**: persist **`SessionEvent`** (including **`UserMessage`** and **`AgentEvent { event: AgentRunResponseMessage }`**); **`Session::resume`** vs **`Session::run`** share the same outward agent item type; tool authorization uses **`reply_tool_auth`** after a gate event (decisions may be supplied out of order per **`call_id`** when multiple tools await)

### Testing Strategy

- **Unit tests**: agent replay and toolbox behavior in **`moray-core`**; transcript and session orchestration in **`moray-sonda`** / **`moray-session`**
- **Integration tests**: workspace `cargo test` (offline mocks; no live model APIs)
- **Interactive use**: **`just dev`** / Tauri desktop app, or **`cargo run -p moray-cli`** for tool sidecar; configure completions in **`<app_data>/settings.toml`** (optional **`env:MORAY_OPENAI_API_KEY`** at turn time)

### Git Workflow

- Conventional commits (repository-level rule); OpenSpec proposals under `openspec/changes/`

## Domain Context

- **Transcript format**: JSONL with a required first-line header `{"moray_transcript_schema":1}` and one **`SondaSessionEventRecord`** (`seq` + **`SessionEvent`**) per subsequent line; unsupported schema versions are rejected

## Important Constraints

## External Dependencies

- **`async-openai`**: used by **`moray-sonda`** for OpenAI-compatible HTTP streaming
