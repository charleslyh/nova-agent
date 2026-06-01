## Context

`desktop/server` 是一个 Tauri 桌面应用内嵌的 loopback HTTP 服务，单 session、SSE 推送、TOML 配置加载与 JSONL transcript 持久化。当前实现集中在 `desktop/server/src/lib.rs`（约 504 行）：

- 顶层公开 API：`start() -> ServerHandle`、`ServerHandle::shutdown()`，前者内部 `tokio::spawn(axum::serve(...))`，后者 `cancel()` + `JoinHandle::await`。
- `Harness` 实现、`AppState`、错误信封、4 个 axum handler、SSE 编码、`reject_unknown_session`、`parse_from_seq` 等全部同文件实现。
- 集成测试通过 `#[cfg(test)] mod tests` 内联，使用 `start()` + `shutdown()` 端到端。

调用方 `desktop/client/src-tauri/src/main.rs` 在 Tauri `setup` 钩子里 `block_on(start())`，在 `RunEvent::ExitRequested` 钩子里 `handle.shutdown().await`，并保留一个 `AtomicBool` 防止重入退出。

约束：

- workspace 编译型语言（Rust 2021），必须能通过 `cargo build` / `cargo test`。
- 不引入新的运行时依赖到 server（首选最小改动）。
- 不破坏现有 server.toml 行为、SSE 事件格式与 HTTP 端点契约。
- 保持 server 的依赖反转：仍然不依赖具体 HTTP SDK 之外的应用层细节。

## Goals / Non-Goals

**Goals:**
- 根 `desktop/server/src/lib.rs` 仅含 `mod` 声明与 `pub use`，无业务实现。
- 按职责拆分实现到独立子模块，单模块尺寸控制在可独立 review 的规模。
- server 公开 API 不再 `tokio::spawn` 任何任务，也不再返回包含 `JoinHandle` / `CancellationToken` 的 handle；改为返回“监听器 + 路由”这种可组合产物，由调用方决定如何 serve。
- desktop client 自己持有 `tokio::spawn` 与 graceful shutdown 逻辑。
- server 库设计上等价于既可被 desktop client 内嵌运行，又可被一个未来的独立可执行 `main()` 直接 serve（共享同一个 `prepare()`）。

**Non-Goals:**
- 不在本次实现独立子进程模式的可执行入口；只确保库 API 形态允许它。
- 不改变 HTTP 端点路径、SSE 事件名、错误码、TOML schema。
- 不重新设计 `Harness` / `AppState` 的字段构成；仅做搬运。
- 不引入新的 crate；保留单 server crate。

## Decisions

### D1：根 lib.rs 只含 `mod` + `pub use`

**决策**：拆分后 `lib.rs` 大体仅这些内容（示意）：

```rust
#![forbid(unsafe_code)]

mod bootstrap;
mod config;
mod error;
mod harness;
mod routes;
mod sse;
mod state;

pub use bootstrap::{prepare, ServerComponents, StartError};
pub use config::{
    config_file_path, load_config, load_config_from, CompletionCredentials, ConfigError,
    ResolvedServerConfig, SERVER_CONFIG_FILE_NAME,
};
```

**Rationale**：让 “lib.rs 是导出表” 成为一个明确的可机器检查的约定（最大行数预算、`cargo expand`/grep 可校验），避免回潮。

**Alternatives**：保留少量胶水函数在 lib.rs，例如 `prepare()` 直接定义于 lib.rs。被否决——会持续诱导新增实现回到 lib.rs。

### D2：子模块划分以“一个抽象一个文件”为原则

```
desktop/server/src/
├── lib.rs                # 仅 mod + pub use
├── config.rs             # 现状保留：TOML 解析与路径
├── state.rs              # AppState（路由的应用级状态）
├── harness.rs            # ServerHarness + CompletionCredentials::openai_chat_completion 扩展
├── error.rs              # ErrorEnvelope、error_json（不再含 session 守卫，见 D7）
├── sse.rs                # parse_from_seq、sse_replay_event、sse_live_event（纯函数）
├── routes.rs             # 4 个 handler + Router 装配函数 build_router(state) -> Router
└── bootstrap.rs          # prepare() / ServerComponents / StartError，绑定监听器并装配状态
```

**Rationale**：

- `state.rs` 与 `harness.rs` 分开是因为 `Harness` 是 session-runtime 的实现者，`AppState` 是 HTTP 路由的入参——它们的依赖方向不同，未来更换 Harness 实现时只动 `harness.rs`。
- `error.rs` 只承载“HTTP 错误响应封装”这一件事（`ErrorEnvelope`、`error_json`）。原本同文件的 `SINGLE_SESSION_ID` / `reject_unknown_session` 不保留——见 D7。
- `sse.rs` 是无状态纯函数，便于独立单测。
- `routes.rs` 同时聚合 4 个 handler 与 `build_router(state)`，避免再嵌套一层 `routes/` 目录（4 个 handler 总行数适中，不必拆到 4 个文件，简单优先）。
- `bootstrap.rs` 是唯一接触 IO（绑定监听器、读取文件、装配 store 与 session）的模块，方便后续加错误类型与日志。

**Alternatives**：
- “按 endpoint 一个文件”：当前 4 个 handler 总规模约 130 行，拆 4 个文件徒增导航成本，被否决。
- “把 `Harness` 实现合并进 `bootstrap.rs`”：与 D1 的“lib.rs 即导出表”原则一致，但 `bootstrap` 模块还要兼任 IO 装配，容易膨胀，被否决。

### D3：新公开 API —— `prepare() -> ServerComponents`

**决策**：以下面这组类型替换现有的 `start` / `ServerHandle`：

```rust
pub struct ServerComponents {
    pub listener: tokio::net::TcpListener,
    pub local_addr: std::net::SocketAddr,
    pub app: axum::Router,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn prepare() -> Result<ServerComponents, StartError> { ... }
```

调用方典型用法：

```rust
let components = moray_desktop_server::prepare().await?;
let local_addr = components.local_addr;
let token = tokio_util::sync::CancellationToken::new();
let shutdown = token.clone();
let join = tokio::spawn(async move {
    let _ = axum::serve(components.listener, components.app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await; })
        .await;
});
```

**Rationale**：

- 把 “选择运行时 / 决定如何 spawn / 何时优雅退出” 全部还给调用方。
- 进程内嵌（desktop client）和独立可执行（未来 `main.rs`）走同一个 `prepare()`；独立可执行只需在 `tokio::main` 下直接 `axum::serve(...).await`。
- 保留同步可访问的 `local_addr`，让 Tauri 的 `get_server_url` 命令实现无需异步状态同步。
- 用具名错误类型 `StartError` 替换原本的 `Result<_, String>`，更可观测，且不破坏调用方使用模式（client 自行 `map_err(|e| e.to_string())`）。

**Alternatives**：
- `pub fn build_app(state: AppState) -> Router` + `pub fn build_state(...) -> AppState`：更细粒度但要求调用方知道 store、session、authorizer 的装配顺序，违反“可复用能力”应当是“可直接 serve 起来”的初衷，被否决（仍可在后续小版本里增量暴露）。
- 让 `prepare()` 直接接受 `TcpListener` 作为参数（不在库里 bind）：未来独立可执行同样要 bind 一次，没必要把 bind 推给调用方；在桌面进程内嵌场景 client 也无理由提供自己的 listener。结论：bind 在库内、spawn 在库外，是关注点最清晰的划分。

### D4：client 侧持有自己的 `ServerHandle`

**决策**：在 `desktop/client/src-tauri/src/` 下新增 `server_supervisor.rs` 模块（或同 main.rs 内联），定义：

```rust
struct ServerHandle {
    local_addr: SocketAddr,
    cancel: CancellationToken,
    join: JoinHandle<()>,
}

impl ServerHandle {
    async fn spawn() -> Result<Self, String> { /* prepare() + tokio::spawn */ }
    async fn shutdown(self) { /* cancel + join.await */ }
}
```

`SharedServer` 持有 `Arc<Mutex<Option<ServerHandle>>>`，对外提供给 Tauri 的 `get_server_url` 与 `RunEvent::ExitRequested` 钩子。

**Rationale**：与既有 `main.rs` 中 `SharedServer` 的接口几乎 1:1 平移，迁移代价最低。同时把生命周期所有权显式归 client。

**Alternatives**：让 server 库提供一个可选的 `feature = "embedded"` 暴露 `start()`。被否决——再次把运行时职责泄漏回库内，违反主目标。

### D5：测试迁移到集成测试目录

**决策**：把 `lib.rs` 内的 4 个 `#[tokio::test]` 迁出到 `desktop/server/tests/http.rs`，使用新的 `prepare()` + 内部 `spawn` 模式（用一个测试本地的 `spawn_for_test(components)` 辅助函数）。`HOME` 隔离与 `TestHomeGuard` 一并迁到该集成测试文件中。

**Rationale**：

- 集成测试本就更适合放在 `tests/` 目录；trim 后 `lib.rs` 也更纯净。
- 通过 `dev-dependencies` 已有的 `reqwest` / `tempfile` 不变。
- 让 server 库本体不再因为测试需要而依赖 `start()` 这种胶水形态。

**Alternatives**：保留 `mod tests` 在 `lib.rs`。被否决——会再次违反 D1。

### D6：错误类型本地化

`StartError` 放在 `bootstrap.rs`，而非 `config.rs` 或新建 `error.rs`：

- `ConfigError` 仍在 `config.rs`（已有），通过 `#[from]` 嵌入 `StartError`。
- HTTP 路由侧的 `ErrorEnvelope` 在 `error.rs`；二者不混淆。

### D7：移除单 session 守卫（`SINGLE_SESSION_ID` / `reject_unknown_session`）

**决策**：删除 `SINGLE_SESSION_ID` 常量、`reject_unknown_session` 助手以及所有 handler 中对路径 `session_id` 的值校验分支。`/sessions/{session_id}/...` URL 形状保留以避免破坏 client；handler 直接消费单 session 实例，不再读取 `Path<String>` / `Path<(String, String)>` 中的 session 段值（用 `Path<_>` 占位丢弃即可）。

**Rationale**：

- server 只 `bind 127.0.0.1:0`；唯一调用方是同进程内嵌的 desktop client；不存在恶意/陌生 client 提交错误 session id 的现实风险面。
- 既有 spec “Single active session HTTP boundary” 明确写过 “the server MUST NOT require the client to supply a session id to identify which session to use”——guard + path 校验属于 spec 的过度防御实现，删除符合既定意图。
- 现状的 guard 同时让我们在维护一个测试 (`unknown_session_returns_404`) 与一段"现实不会触发"的代码，违反 YAGNI 与“简单方案优先”的 user rule。
- 删除后 `error.rs` 的职责单一为“HTTP 错误响应编码”，进一步贴合 D2 的"一个抽象一个文件"原则。

**回滚条件（什么时候应当再次引入类似守卫）**：

- `add-sessions-manager` 落地多 session 之后，路由层需要把 path 中的 `session_id` 当作真实路由键时，由 sessions-manager 的路由层提供等价或更强的守卫（例如 "session not found" → 404），而不再由 `error.rs` 兜底。
- 如果 server 未来暴露到非 loopback 地址，需要先增加鉴权/路由准入层；那时守卫由认证中间件提供，仍不回到 `error.rs`。

**Alternatives**：
- 保留 guard 但移到 `routes.rs` 顶部：仍维护"零现实风险"的代码，被否决。
- 把 URL 改为 `/messages` 无 session 段：客户端 URL 形状改动较大，不必要的 BREAKING，被否决。

**测试影响**：删除 `unknown_session_returns_404` 测试；其它三个测试（`reset_endpoint_accepts_default_session`、`tool_auth_without_pending_call_returns_not_found_404`、`events_stream_emits_replay_then_live_start_after_reset`）继续保留——它们检验的是 session 业务行为而非 path 守卫。

## Risks / Trade-offs

- [移除 `start()` 与 `ServerHandle` 是 BREAKING API] → 仅此 workspace 内 `desktop/client` 与 server 自身测试使用，迁移点已知且数量很小；通过本变更原子化完成。
- [拆模块后跨文件可见性需调整] → 把现有 `pub(crate)` 与 `pub` 显式标好；保持 `state::AppState`、`harness::ServerHarness` 仅 crate 内可见，避免外部依赖私有装配细节。
- [`prepare()` 内部 bind 失败但 config 已加载，无法回滚配置副作用] → 配置加载是纯读取，无副作用，可忽略。
- [client 自管 `JoinHandle` 后，若 `RunEvent::ExitRequested` 路径异常会泄漏任务] → 行为与现状一致（现状也是 client 触发 `shutdown()`），不在本次扩展范围。
- [SSE 助手成为 `pub(crate)` 后跨 crate 重用受限] → 故意收紧；若未来确需被独立可执行复用，再通过 `prepare()` 链路自然复用，不需要把内部纯函数公开。
- [测试改用集成测试目录，构建时间可能略增] → 仅一个 `tests/http.rs` 文件，影响可忽略；同时让 `lib.rs` 不再受测试依赖污染（实际依赖 `reqwest` / `tempfile` 仍走 `dev-dependencies`）。

## Migration Plan

1. 引入子模块文件并把代码原样搬运（编译应仍然通过，但 `start()`/`ServerHandle` 暂时保留为薄包装委托到 `bootstrap`，以便分步验证）。
2. 把测试迁到 `tests/http.rs` 并使用即将上线的 `prepare()` API（如还未公开则先用 `pub(crate)` + `#[path]` 重导出方式过渡——一次性提交时不必要）。
3. 删除 `start()` / `ServerHandle`，公开 `prepare()`。
4. 在 `desktop/client/src-tauri/` 引入 `server_supervisor`（或内联），消费 `prepare()`，定义 client 内部的 `ServerHandle`；为 `tokio-util` 增加依赖。
5. `cargo build -p moray-desktop-server -p moray-desktop-client` 编译验证；`cargo test -p moray-desktop-server` 跑通端到端测试。
6. 归档时按 delta 更新 `openspec/specs/server/spec.md`。

回滚：本变更是同一 PR 内的纯重构 + API 迁移，回滚即 revert PR。

## Open Questions

- 是否需要把 `local_addr` 通过 `mpsc::Sender` 等异步通道暴露给 client，以避免 `SharedServer` 持有 `Arc<Mutex<Option<ServerHandle>>>`？  
  当前结论：保持现有锁模式即可，避免大改 Tauri 命令边界。
- 是否需要在本次提供一个最小可运行的独立可执行 `bin/`?  
  当前结论：不需要。库形态足以验证“可独立进程化”的设计意图；待真正需要再添。
