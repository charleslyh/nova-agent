## 1. Transcript Store Subscription Foundation

- [x] 1.1 Add a sequence-based subscribe API on `JsonlTranscriptStore` (returns `UnboundedReceiver<StoredSessionRecord>`); records carry an explicit `seq` so consumers can resume from a known position.
- [x] 1.2 Implement multi-subscriber registration in `JsonlTranscriptStore` using unbounded channels.
- [x] 1.3 Implement publish fan-out with failed-send subscriber removal to keep subscriber registry clean.
- [x] 1.4 Add focused tests for subscriber lifecycle (multiple subscribers, dropped subscriber cleanup, independent delivery).
- [x] 1.5 Enforce `start_seq` lower-bound filtering in `subscribe(start_seq)` (subscribers store their own `start_seq` and `publish_delta` skips records with `seq < start_seq`); covered by `subscriber_does_not_receive_events_below_start_seq`.

## 2. Chat View Single-Source Rendering Flow

- [x] 2.1 Update `chat.rs` replay flow to compute `start_seq = last_loaded_seq + 1` from `JsonlTranscriptStore::load()` (returned as `next_seq`).
- [x] 2.2 Create a store subscriber after replay and route all chat rendering through subscriber-delivered delta events.
- [x] 2.3 Refactor direct `session.post` stream rendering path so operation calls mutate session state but do not directly drive view rendering; `session.post` now accepts an `Arc<dyn SessionEventSink>` (chat demo passes `NoopSink` because rendering is fed by the subscriber).
- [x] 2.4 Replace ad-hoc REPL with an `Idle / Running / Exit` state machine in `main`, with `repl_once` (input handling) and `process_next_session_event` (subscriber rendering) extracted as helpers.
- [x] 2.5 Introduce `SessionEventKind::Reset` so `/clear` emits a single delta the view renders via `ChatView::clear`, removing the direct `view.reset()` side path.

## 3. Verification and Regression Coverage

- [x] 3.1 Subscriber-boundary correctness is covered by transcript-store tests (`subscribers_receive_same_future_events`, `subscriber_does_not_receive_events_below_start_seq`); demo `chat.rs` keeps replay (`view.replay`) and live rendering (`process_next_session_event`) on disjoint paths so duplicate replay rendering is structurally avoided.
- [x] 3.2 `subscribers_receive_same_future_events` exercises the `SessionStore::append` → subscriber render path through `JsonlTranscriptStore`; `session_cancel.rs` continues to validate that detached `post` runs persist agent events through the same store, which now also drives subscribers.
- [x] 3.3 `cargo test --workspace` runs green for `nova-core`, `nova-builtin`, `nova-sessions`, and `demo`, including the new subscriber tests.
