## ADDED Requirements

### Requirement: Multi-crate workspace layout for moray agent runtime (initial phase)

The repository SHALL provide a Rust workspace whose members include **`moray-core`** and a **non-publishable** demo package (the demo manifest SHALL set `publish = false`). **`moray-session` MUST NOT be a required workspace member in this phase.**

The dependency graph SHALL allow **demo → `moray-core`** only among these deliverables; **`moray-core` MUST NOT** depend on the demo crate.

#### Scenario: Core builds without session or demo

- **WHEN** building the `moray-core` crate alone
- **THEN** the build MUST succeed without depending on any future `moray-session` crate or the demo crate

### Requirement: Chat completion abstraction for the agent loop

`moray-core` SHALL define a `CompletionProvider` (or equivalently named) trait that abstracts **OpenAI chat-completion-compatible** behavior used by the ReAct loop: streaming assistant output and tool invocations shaped so that a single mock can stand in for multiple vendors without changing the loop.

The trait SHALL expose capability information needed for recovery via **`CompletionCapabilities`** (for example **`prefill_supported`**: assistant-message prefill or equivalent “continue final assistant message” semantics).

Streaming chunks SHALL use a shape that allows **incremental assistant text** and **finalized tool calls emitted individually** before end-of-turn (as implemented by **`CompletionChunk`**: text deltas, per-item tool calls, then **finished**).

LLM interactions that are **not** chat completions (for example image generation protocols) are out of scope for this trait; integrating them MUST NOT expand this trait’s surface—adapters SHOULD bridge such results into chat messages or tool outcomes instead.

#### Scenario: Capability is queryable without network

- **WHEN** a harness constructs a mock `CompletionProvider` implementation
- **THEN** the harness MUST be able to read the prefill/continue capability flag without performing HTTP calls

### Requirement: Toolbox abstraction

`moray-core` SHALL define a `Toolbox` trait that supports listing invokable tools and executing a tool call with structured arguments, returning results suitable for feeding back into the reasoning loop.

#### Scenario: Mock toolbox lists and invokes without I/O

- **WHEN** tests register a mock toolbox with deterministic tool definitions
- **THEN** the core loop MUST list those tools and receive deterministic mock results on invocation

### Requirement: Dynamic polymorphism for CompletionProvider and Toolbox

`moray-core` SHALL support using **`CompletionProvider`** and **`Toolbox`** behind **trait objects** (for example `Box<dyn CompletionProvider + Send + Sync>` and `Box<dyn Toolbox + Send + Sync>`, or `Arc` equivalents) so harnesses can select implementations at runtime without changing the concrete `Agent` type parameters. The library SHALL provide forwarding implementations for `Box` and `Arc` wrapping these trait objects.

#### Scenario: Agent runs with boxed model and toolbox

- **WHEN** a test constructs an `Agent` with `Box<dyn CompletionProvider + Send + Sync>` and `Box<dyn Toolbox + Send + Sync>` built from mock types
- **THEN** `invoke` MUST complete successfully and emit the expected domain events

### Requirement: ReAct loop with explicit state machine

`moray-core` SHALL implement a ReAct-style agent loop as an explicit state machine driven by user input, chat completion stream chunks, toolbox results, and authorization decisions, emitting an ordered sequence of domain events.

#### Scenario: Tool call reaches authorization gate

- **WHEN** the completion stream yields a tool call that requires external authorization
- **THEN** the state machine MUST transition to an awaiting-decision state and emit an event that carries sufficient data for a harness to prompt and persist a decision

### Requirement: Invoke and resume share the same outward event stream shape

The public API for starting a new user turn (`invoke` or equivalent) and for resuming from persisted history (`resume` or equivalent) SHALL expose the same event item type and the same high-level streaming contract, so a single consumer can process both paths.

#### Scenario: Same stream consumer for invoke and resume

- **WHEN** the CLI handles an unfinished QA section and later handles a REPL turn
- **THEN** both paths MUST be able to reuse the same stream-processing logic (e.g. one demo-local `run_*` helper) so `invoke` and `resume` are consumed consistently without separate incompatible match arms

### Requirement: Recovery from persisted events after external suspension

If execution stops while awaiting tool-call authorization (or another harness-controlled suspension encoded in events), the agent MUST be able to rebuild deterministic state from persisted events and continue execution when resumed.

#### Scenario: Crash after tool request event persisted

- **WHEN** the transcript contains a tool-call request event and a subsequent authorization outcome event is not yet persisted
- **THEN** after restart, `resume` MUST reproduce the same post-request state and, once the harness supplies the decision, continue without re-requesting chat completion for the same tool call unless policy requires it

### Requirement: Non-resumable completion turns discard uncommitted output

If the `CompletionProvider` implementation does not support assistant prefill/continue semantics, and the process terminates abnormally after partial assistant output for the current turn without reaching a persisted stable checkpoint for that completion turn, recovery MUST discard uncommitted partial assistant output for that turn and re-run chat completion from the last stable checkpoint.

#### Scenario: Mock completion declares no prefill and simulates interrupted stream

- **WHEN** the `CompletionProvider` implementation reports no prefill support and the test simulates crash mid-stream before turn commitment
- **THEN** after `resume`, the subsequent completion invocation MUST NOT treat partial assistant text from the interrupted attempt as valid context (verification via mock call counting or injected markers)
