## MODIFIED Requirements

### Requirement: Shared stream consumption for resume and run

The **`chat`** example SHALL delegate processing of streaming session events from both **`resume`** and **`run`** to a single shared implementation (for example **`drain_session_stream`**) that handles text output, tool-call lifecycle rendering, and tool-authorization prompts consistently, using **`SessionEvent`** as the outward stream item type.

Tool-authorization prompting SHALL be triggered by destructuring the nested pattern **`SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, .. } } }`**. Assistant text and tool-call lifecycle rendering SHALL continue to dispatch on **`SessionEventKind::AgentEvent { event }`** using the simplified **`AgentRunResponseMessage`** variants: **`ChatResponse { chunk }`**, **`ToolCall { event: ToolboxEvent::Requested { .. } }`**, **`ToolCall { event: ToolboxEvent::RequestingPermission { .. } }`**, **`ToolCall { event: ToolboxEvent::Started { .. } }`**, **`ToolCall { event: ToolboxEvent::Finished { .. } }`**, and **`Finished { kind }`**. `Requested` and `RequestingPermission` MAY be rendered as no-ops in minimal UIs (the prompt workflow itself is what surfaces permission state).

Archived transcript replay for completed sections SHOULD use the **same** user-visible patterns as interactive turns (no special **`[replay:]`** prefixes); pending resume SHOULD use the same turn runner as new **`run`** calls.

#### Scenario: Authorization prompt is driven by the nested AgentEvent pattern

- **WHEN** a live turn emits a `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, .. } } }`
- **THEN** the shared drainer MUST prompt the user for approval (recovering tool name / arguments either from the preceding `ToolCall { event: Requested { call_id, name, arguments } }` event or from its own tool cache populated from the earlier `ChatResponse { chunk }` tool-call chunk) and forward the decision via `Session::reply_toolcall_permission(&call_id, serde_json::Value::Bool(allowed))`

#### Scenario: No duplicated match loop in main

- **WHEN** reviewing the example source for pending-section handling versus the REPL loop
- **THEN** the `SessionEvent` matching and side-effect dispatch MUST be centralized in one reusable unit
