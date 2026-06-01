## Why

`desktop/server/src/lib.rs` 当前是一个 500+ 行的“上帝文件”：路由处理、SSE 编码、HTTP 错误编码、`Harness` 实现、`AppState` 装配、TCP 监听绑定与 Tokio task 生命周期管理混杂在同一个模块中，未来要新增端点或调整运行模式时改动半径过大。

同时，server 库主动 `tokio::spawn` 任务、自带 `ServerHandle::shutdown()` 强制把进程内嵌运行模式写死在库里，导致：

- desktop client 与 server 的运行时生命周期紧耦合，client 没有任何 hook 去控制任务调度策略。
- 未来要让 server 在独立子进程或独立可执行文件中运行时，必须重写库的入口才能复用。

本次变更同时解决两个问题：把单体 `lib.rs` 按职责拆成子模块；把“在调用方运行时上 `spawn` 一个 axum task + 用 `CancellationToken` 退出”的代码迁回到 desktop client，由调用方持有 `ServerHandle`。

## What Changes

- 将 `desktop/server/src/lib.rs` 的实现按职责拆分为子模块（`state`、`harness`、`routes`、`sse`、`error`、`bootstrap`），根 `lib.rs` 只保留 `mod` 声明与公开再导出，无函数/类型实现。
- **BREAKING**：移除 server 库的 `start() -> ServerHandle` 与 `ServerHandle::shutdown()`。server 改为暴露可复用装配能力：`prepare()` 返回已绑定的 `TcpListener`、`local_addr` 与 `axum::Router`（统一打包为 `ServerComponents`），不再 `tokio::spawn` 任何任务。
- 将 “在 Tokio 任务里 serve + 通过 `CancellationToken` 做 graceful shutdown” 的代码迁移到 `desktop/client/src-tauri/src/`，并由 client 内部定义自己的 `ServerHandle`（含 `local_addr`、`CancellationToken`、`JoinHandle`）。
- 顺势移除单 session 阶段冗余的 `SINGLE_SESSION_ID` 常量与 `reject_unknown_session` 守卫：server 只绑定 loopback、唯一调用方是同进程内嵌 client，请求被认为可信；handler 不再校验路径上的 `session_id` 值。`/sessions/{session_id}/...` URL 形状保持不变以避免破坏 client。未来 multi-session 路由层落地（`add-sessions-manager`）时再补回必要的容错。
- server 集成测试改为复用 client 同样的“先 `prepare()` 再 spawn”模式（通过 server 自带的 `dev-dependencies`-only 测试辅助函数完成 spawn，不进入 server 的 public API），并删除 `unknown_session_returns_404` 测试。
- 在 server crate 中显式声明“运行模式与生命周期归调用方所有”的契约，从而让未来以独立进程启动 server 时只需提供新的可执行入口（消费同一个 `prepare()`），不必改动库代码。

## Capabilities

### New Capabilities
（无新增 capability。本次为既有 server capability 的实现重构与运行时职责再分配。）

### Modified Capabilities
- `server`: 重写 `Server crate is consumed as a library` 与 `In-process server lifecycle with graceful shutdown` 两条 requirement，使其表述“暴露可复用装配 API 而不自带 task spawn / 自带 `ServerHandle`”，并新增 `Library lib.rs contains no implementation` 要求。

## Impact

- 代码：`desktop/server/src/lib.rs` 拆分为多个子模块；新增 `desktop/server/src/{state,harness,routes,sse,error,bootstrap}.rs`；`desktop/client/src-tauri/src/main.rs` 接管 `tokio::spawn` 与 `CancellationToken` 生命周期，新增 client 内部的 `server_supervisor` 模块持有 `ServerHandle`。删除 `SINGLE_SESSION_ID` / `reject_unknown_session` 与所有 handler 中的 session id 守卫分支。
- 公开 API：移除 `pub use` 的 `ServerHandle` 与 `start`；新增 `pub use bootstrap::{prepare, ServerComponents, StartError}`。
- 测试：`desktop/server/src/lib.rs` 内的集成测试迁出到 `desktop/server/tests/http.rs`，使用 `prepare()` 而非 `start()` 启动；删除 `unknown_session_returns_404` 测试；client 端如果有冒烟测试需同步调整（当前 `main.rs` 在 `#[cfg(test)] fn main() {}`，无 client 侧 server 启动测试需要变更）。
- 依赖：`desktop/client/src-tauri/Cargo.toml` 新增 `tokio-util`（`CancellationToken`）依赖；`desktop/server/Cargo.toml` 可继续保留 `tokio-util` 以服务于测试辅助函数。
- 文档：`openspec/specs/server/spec.md` 在归档时按 delta 更新；`server.toml.example` 与 `~/.moray/` 行为不受影响。
