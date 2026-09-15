## MODIFIED Requirements

### Requirement: Shared stream consumption for resume and run

The **`chat`** example SHALL delegate processing of streaming agent events from both **`resume`** and **`run`** to a single shared implementation (for example **`run_agent_turn`**) that handles text output and tool authorization prompts consistently, using **`AgentRunResponseMessage`** (including **`ChatResponseChunk`** for model output) as the agent stream item type.

Archived transcript replay for completed sections SHOULD use the **same** user-visible patterns as interactive turns (no special **`[replay:]`** prefixes); pending resume SHOULD use the same turn runner as new **`run`** calls.

#### Scenario: No duplicated match loop in main

- **WHEN** reviewing the example source for pending-section handling versus the REPL loop
- **THEN** the **`AgentRunResponseMessage`** matching and side-effect dispatch MUST be centralized in one reusable unit

### Requirement: Integration tests with mock chat completion and toolbox

The workspace SHALL include integration tests under **`demo/tests/*.rs`** using **`demo::mock`** (**`ChatCompletion`** + **`Toolbox`** doubles) against **`nova_core`**. Tests SHALL cover recovery after authorization suspension and replay behavior for **`!prefill_supported`** (including **`replay`** / **`agent`** integration as implemented). **`nova-core`** SHALL NOT depend on **`demo`**.

#### Scenario: Tests pass without external services

- **WHEN** CI runs **`cargo test`** for the affected crates
- **THEN** these integration tests MUST pass without network access
