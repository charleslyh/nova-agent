## Context

Current `chat.rs` rendering is tied to the stream returned by `session.post`, while replay and other session operations also affect session state. This creates split ownership of UI state and makes resume/replay behavior harder to reason about. The target architecture is to make transcript-store delta subscription the single source for view updates so the same model can later support web navigation, resumed rendering, and consistent state restoration.

## Goals / Non-Goals

**Goals:**
- Separate session operations (`post`, `reset`, `resume`, etc.) from view rendering.
- Make chat view consume only store-subscriber delta events after replay.
- Support sequence-based subscription (`last_seq + 1`) to avoid duplicate rendering.
- Support multiple subscribers in `JsonlTranscriptStore` with automatic stale-subscriber cleanup.
- Keep runtime flow simple enough for CLI use without introducing complex orchestration.

**Non-Goals:**
- Introducing async UI frameworks or a full event bus beyond current store/session boundaries.
- Redesigning transcript persistence format.
- Changing model/tool protocol semantics in `moray_core`.

## Decisions

- Use transcript store as the view-facing event source.
  - Rationale: centralizes state deltas from all session operations and avoids operation-specific render paths.
  - Alternative considered: keep direct `session.post` stream rendering and manually merge replay/reset deltas in UI. Rejected due to multi-source complexity and higher risk of divergence.

- Subscribe from replay boundary (`last_loaded_seq + 1`).
  - Rationale: replay is rendered once from loaded history, and live rendering starts exactly after that boundary.
  - Alternative considered: subscribe from `0` and filter in UI. Rejected because it duplicates replay logic and increases UI responsibility.

- Implement multi-subscriber fan-out in `JsonlTranscriptStore` using unbounded channels.
  - Rationale: unbounded channel simplifies producer side and avoids backpressure coupling between core operations and rendering consumers.
  - Alternative considered: bounded channels with blocking or drop policies. Rejected for this stage to keep the demo path straightforward and avoid deadlocks/complex tuning.

- Remove failed subscribers eagerly on send error.
  - Rationale: dropped receivers should not accumulate in store state; cleanup on send failure is simple and deterministic.
  - Alternative considered: periodic GC sweep. Rejected as unnecessary complexity for current scale.

- Drive `main` with a small `Idle / Running / Exit` state machine instead of a `tokio::select!` multiplexer.
  - Rationale: the demo strictly alternates between "wait for one user input" and "drain subscriber events until `Finished`", so a state machine with two helpers (`repl_once`, `process_next_session_event`) expresses the flow without `select!`-related cancellation-safety pitfalls.
  - In `Running` state, subscriber-delivered `Blocked` tool-call events trigger an inline authorization prompt (`view.prompt_tool_authorization` + `session.reply_toolcall_permission`) and the loop stays in `Running`, removing the need for a separate `AWAITING_AUTH` state.
  - Alternative considered: a single `tokio::select!` loop multiplexing stdin and subscriber events. Rejected for now because the strict alternation makes multiplexing unnecessary and `select!` would force us to handle stdin cancellation and event-priority concerns we currently do not need.
  - Alternative considered: split REPL and renderer into independent tasks with shared state. Rejected for now because task coordination adds overhead without immediate product benefit.

- Expose typed delta events from transcript store, instead of generic event envelopes.
  - Rationale: typed deltas keep session-state semantics explicit, reduce adapter ambiguity, and align with single-source-of-truth rendering.
  - Alternative considered: generic envelopes for broader reuse. Rejected because it shifts interpretation complexity to consumers and weakens contract clarity for state tracking.

## Risks / Trade-offs

- Unbounded channels can grow if consumers stall for long periods -> Mitigation: assume consumers are responsible for timely event handling; keep producer path non-blocking and cleanup dropped subscribers promptly.
- Single-loop multiplexing may still have readability concerns -> Mitigation: extract helper functions for input handling and delta rendering to keep loop minimal.
- Sequence boundary bugs can cause missing or duplicate messages -> Mitigation: add integration tests around replay boundary and live-post transitions.
- Multi-subscriber management introduces shared mutable state -> Mitigation: encapsulate subscriber registry in store internals and cover add/remove lifecycle with tests.

## Migration Plan

- No external migration is required.
- Update `chat.rs` to subscribe after replay and switch rendering source.
- Extend `JsonlTranscriptStore` with subscribe API and multi-subscriber lifecycle logic.
- Add tests for sequence offset correctness and dropped-subscriber cleanup.
- If regression appears, temporary rollback path is to disable subscriber-based render flow in demo and restore direct stream rendering.

## Runtime State Machine (Minimal Sketch)

```text
   ┌──────────────────────────────┐
   │          bootstrap           │
   │ load() -> (records, next_seq)│
   │ view.replay(records)         │
   │ subscribe(next_seq)          │
   └──────────────┬───────────────┘
                  ▼
            ┌──────────┐  user chat / /clear   ┌──────────┐
            │   Idle   │ ─────────────────────▶│ Running  │
            │ stdin in │                       │ subscr.  │
            │ repl_once│ ◀─────────────────────│ recv     │
            └────┬─────┘   AgentResponse::     └────┬─────┘
                 │         Finished                 │ subscriber closed
                 │         (back to Idle)           │
                 ▼                                  ▼
            ┌──────────┐                       ┌──────────┐
            │   Exit   │ ◀─────────────────────│   Exit   │
            └──────────┘   empty input / EOF   └──────────┘
```

- Bootstrap: `JsonlTranscriptStore::load` returns `(records, next_seq)`; `ChatView::replay` rebuilds the visible terminal output from history, then `store.subscribe(next_seq)` opens the live delta stream.
- `Idle` (`repl_once`): read one stdin line.
  - empty / EOF → `Exit`.
  - `/clear` → `session.reset()` (emits `SessionEventKind::Reset`) → `Running` to drain that delta.
  - `/<other>` → `view.render_unknown_command` and stay in `Idle`.
  - chat message → `session.post(input, NoopSink)` → `Running`.
- `Running` (`process_next_session_event`): block on `subscriber.recv()`.
  - `TurnAccepted`: stay `Running` (the user's own line was already echoed by `repl_once`; this branch keeps state-machine symmetry).
  - `AgentResponse { msg }`: route through `ChatView::process_agent_response_message`. If `msg` is `ToolCall { event: Blocked { call_id, .. } }`, prompt inline via `view.prompt_tool_authorization` and call `session.reply_toolcall_permission`. If `msg` is `Finished { .. }`, return to `Idle`; otherwise stay `Running`.
  - `Reset`: `view.clear()` and return to `Idle`.
  - subscriber channel closed → `Exit`.

This keeps a single rendering source: anything that mutates session state (post, reset, future operations) is observed by the view exclusively through the subscriber stream.
