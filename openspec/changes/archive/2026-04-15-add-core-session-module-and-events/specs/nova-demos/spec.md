> Historical note (2026-04-29): this archived spec references the old layout where OpenAI lived behind the `nova-core` `openai` feature. Current architecture uses `nova-builtin` and `nova_builtin::completions::OpenAIChatCompletion`.

## MODIFIED Requirements

### Requirement: chat example

The **`demo`** crate SHALL provide a **`chat`** example that loads a transcript, replays completed sections to stdout, initializes a session runtime with a **`ChatCompletion`** suitable for interactive use, resumes any pending work via **`nova_core::Session`**, then enters a REPL that triggers new user turns through the same session runtime.

The example SHALL use **`nova_core::OpenAIChatCompletion`** (dependency **`nova-core`** with feature **`openai`**) as **`ChatCompletion`**, constructed via **`demo::chat_completion_from_env`**, which reads **`NOVA_OPENAI_API_KEY`**, **`NOVA_OPENAI_BASE_URL`**, and **`NOVA_OPENAI_MODEL`**, and SHALL build **`ChatCompletionCapabilities`** from the environment (including optional **`NOVA_OPENAI_PREFILL_SUPPORTED`**) as documented on **`demo::chat_completion_builder`** (re-exported constants **`ENV_*`** from **`demo::`** root). A **`demo`**-level **mock toolbox** (for example **`EchoToolbox`**) MAY be used without a separate tools backend.

Transcript loading and persistence adapters continue to live in **`demo`**, while session lifecycle orchestration (bootstrap/invoke/resume/reset) SHALL be delegated to **`nova_core::Session`**.

#### Scenario: Example documents required environment variables

- **WHEN** reading **`chat`** module docs or **`demo::ENV_*`** / **`chat_completion_builder`**
- **THEN** documentation MUST list **`NOVA_OPENAI_API_KEY`**, **`NOVA_OPENAI_BASE_URL`**, and **`NOVA_OPENAI_MODEL`** as required for the live client, and optional **`NOVA_OPENAI_PREFILL_SUPPORTED`** for **`ChatCompletionCapabilities::prefill_supported`**

#### Scenario: Workspace tests stay offline

- **WHEN** CI runs **`cargo test`** for the workspace
- **THEN** tests MUST pass without network access or live model APIs (mock **`ChatCompletion`** / **`Toolbox`** in **`demo::mock`** or test-only doubles)
