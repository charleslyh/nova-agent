> Historical note (2026-04-29): this archived spec references the old layout where OpenAI lived behind the `moray-core` `openai` feature. Current architecture uses `moray-builtin` and `moray_builtin::completions::OpenAIChatCompletion`.

## MODIFIED Requirements

### Requirement: chat example

The **`demo`** crate SHALL provide a **`chat`** example that loads a transcript, replays completed sections to stdout, initializes a session runtime with a **`ChatCompletion`** suitable for interactive use, resumes any pending work via **`moray_core::Session`**, then enters a REPL that triggers new user turns through the same session runtime.

The example SHALL use **`moray_core::OpenAIChatCompletion`** (dependency **`moray-core`** with feature **`openai`**) as **`ChatCompletion`**, constructed via **`demo::chat_completion_from_env`**, which reads **`MORAY_OPENAI_API_KEY`**, **`MORAY_OPENAI_BASE_URL`**, and **`MORAY_OPENAI_MODEL`**, and SHALL build **`ChatCompletionCapabilities`** from the environment (including optional **`MORAY_OPENAI_PREFILL_SUPPORTED`**) as documented on **`demo::chat_completion_builder`** (re-exported constants **`ENV_*`** from **`demo::`** root). A **`demo`**-level **mock toolbox** (for example **`EchoToolbox`**) MAY be used without a separate tools backend.

Transcript loading and persistence adapters continue to live in **`demo`**, while session lifecycle orchestration (bootstrap/invoke/resume/reset) SHALL be delegated to **`moray_core::Session`**.

#### Scenario: Example documents required environment variables

- **WHEN** reading **`chat`** module docs or **`demo::ENV_*`** / **`chat_completion_builder`**
- **THEN** documentation MUST list **`MORAY_OPENAI_API_KEY`**, **`MORAY_OPENAI_BASE_URL`**, and **`MORAY_OPENAI_MODEL`** as required for the live client, and optional **`MORAY_OPENAI_PREFILL_SUPPORTED`** for **`ChatCompletionCapabilities::prefill_supported`**

#### Scenario: Workspace tests stay offline

- **WHEN** CI runs **`cargo test`** for the workspace
- **THEN** tests MUST pass without network access or live model APIs (mock **`ChatCompletion`** / **`Toolbox`** in **`demo::mock`** or test-only doubles)
