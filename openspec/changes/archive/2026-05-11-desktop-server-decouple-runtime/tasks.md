## 1. Server 子模块拆分

- [x] 1.1 新建 `desktop/server/src/state.rs`，迁入 `AppState`（`pub(crate)` 字段，crate 内可见）
- [x] 1.2 新建 `desktop/server/src/harness.rs`，迁入 `ServerHarness` 与 `CompletionCredentials::openai_chat_completion` 扩展 `impl`
- [x] 1.3 新建 `desktop/server/src/error.rs`，迁入 `ErrorEnvelope`、`error_json`（不再包含 `SINGLE_SESSION_ID` 与 `reject_unknown_session`，见 1.5b）
- [x] 1.4 新建 `desktop/server/src/sse.rs`，迁入 `parse_from_seq`、`sse_replay_event`、`sse_live_event`（以及未使用的 `_tool_blocked_example` 函数：若仍想保留则迁入并保持 `#[allow(dead_code)]`，否则删除）
- [x] 1.5 新建 `desktop/server/src/routes.rs`，迁入 4 个 handler（`post_message`、`post_tool_auth`、`post_reset`、`get_events`）以及对应的请求/查询 DTO，并新增 `pub(crate) fn build_router(state: AppState) -> Router`（含 CORS 层）
- [x] 1.5b 在迁入 handler 的同时，删除 `SINGLE_SESSION_ID` 常量与 `reject_unknown_session` 函数；从每个 handler 顶部移除 `if let Some(response) = reject_unknown_session(&session_id) { return response; }` 这一分支；将 `Path(session_id): Path<String>` 改为 `Path(_): Path<String>`（`get_events` 同理；`post_tool_auth` 的 `Path((session_id, call_id))` 改为 `Path((_, call_id))`）
- [x] 1.6 新建 `desktop/server/src/bootstrap.rs`，定义 `pub struct ServerComponents`、`pub enum StartError`（`thiserror`）、`pub async fn prepare() -> Result<ServerComponents, StartError>`，承担：建目录、`load_config`、构造 store/authorizer/harness/session/state、`TcpListener::bind("127.0.0.1:0")`、`local_addr`、`build_router(state)` 装配
- [x] 1.7 修改 `desktop/server/src/lib.rs`：删除原全部实现，只保留 `#![forbid(unsafe_code)]` + `mod` 声明（`bootstrap`、`config`、`error`、`harness`、`routes`、`sse`、`state`）+ `pub use bootstrap::{prepare, ServerComponents, StartError}` 与现有 `pub use config::{...}`
- [x] 1.8 调整各模块可见性：`AppState`、`ServerHarness`、`error::*`、`sse::*`、`routes::build_router` 均为 `pub(crate)`，不出现在 server 公开 API
- [x] 1.9 运行 `cargo build -p moray-desktop-server` 验证拆分后编译通过

## 2. 移除旧的运行时入口

- [x] 2.1 在 `desktop/server/src/lib.rs` 中确认已不再 `pub use` `start` 或 `ServerHandle`
- [x] 2.2 全工作区搜索 `moray_desktop_server::start` 与 `moray_desktop_server::ServerHandle`，确认仅 `desktop/client/src-tauri` 与 server 内部测试有引用（接下来的步骤会替换它们）

## 3. 集成测试迁移

- [x] 3.1 新建 `desktop/server/tests/http.rs`，把原 `lib.rs` 内 `mod tests` 的代码迁出：`HOME_LOCK`、`TestHomeGuard`、`minimal_server_toml`、`write_default_moray_config`，以及保留的 3 个 `#[tokio::test]`（`reset_endpoint_accepts_default_session`、`tool_auth_without_pending_call_returns_not_found_404`、`events_stream_emits_replay_then_live_start_after_reset`）。**删除** `unknown_session_returns_404` 测试，原因详见 design.md D7
- [x] 3.2 在 `tests/http.rs` 中新增 `async fn spawn_for_test(components: ServerComponents) -> (SocketAddr, CancellationToken, JoinHandle<()>)` 辅助函数，复用与 client 相同的 `tokio::spawn` + `with_graceful_shutdown` 模式
- [x] 3.3 修改 `start_test_server()` 改为：`prepare().await?` → `spawn_for_test(...)`；测试结束后 `cancel()` + `join.await`
- [x] 3.4 由于 `tests/http.rs` 在 crate 外部，原本 `pub(crate)` 的 `config::SERVER_CONFIG_FILE_NAME` 已是 `pub`，无需调整；`desktop_transcript_path` 仍是 `pub(crate)`——在测试中改为通过同样的 `HOME` 计算 transcript 路径（与 server 内部规则一致），避免把内部 API 改公开
- [x] 3.5 运行 `cargo test -p moray-desktop-server` 确认保留的 3 个端到端测试全部通过

## 4. desktop client 接管运行时生命周期

- [x] 4.1 在 `desktop/client/src-tauri/Cargo.toml` 的 `[dependencies]` 增加 `tokio-util = "0.7"`，并为 `tokio` 增加 `rt-multi-thread` 等已有 feature 不动（仅核对当前足够运行 `tokio::spawn` 与 `JoinHandle`）
- [x] 4.2 在 `desktop/client/src-tauri/src/` 下新增 `server_supervisor.rs`（或在 `main.rs` 内联），定义 client 私有的 `struct ServerHandle { local_addr, cancel: CancellationToken, join: JoinHandle<()> }` 与 `impl ServerHandle { async fn spawn() -> Result<Self, String>; async fn shutdown(self); }`
- [x] 4.3 `ServerHandle::spawn` 实现：调用 `moray_desktop_server::prepare().await.map_err(|e| e.to_string())?`，得到 `ServerComponents`，记录 `local_addr`，新建 `CancellationToken`，`tokio::spawn` 一个 future，内部 `axum::serve(listener, app).with_graceful_shutdown(token.clone().cancelled_owned()).await`，返回 `ServerHandle`
- [x] 4.4 修改 `main.rs`：将 `use moray_desktop_server::{start, ServerHandle}` 改为引入 client 私有的 `ServerHandle`；`SharedServer.handle` 仍是 `Arc<Mutex<Option<ServerHandle>>>`，setup 钩子中 `block_on(ServerHandle::spawn())`，退出钩子中 `handle.shutdown().await`
- [x] 4.5 `get_server_url` 命令实现保持不变，仍读取 `handle.local_addr`
- [x] 4.6 运行 `cargo build -p moray-desktop-client`（即 `desktop/client/src-tauri/Cargo.toml` 对应 package）验证 client 编译通过

## 5. 终端验证

- [x] 5.1 运行 `cargo build --workspace` 验证整库编译通过
- [x] 5.2 运行 `cargo test -p moray-desktop-server` 验证服务器测试全过
- [x] 5.3 `rg --hidden -n "tokio::spawn|JoinHandle|CancellationToken" desktop/server/src` 应仅匹配 `tests/` 路径外的 0 项（library 中不应再有任何运行时 spawn / cancellation 痕迹）；若有则修正
- [x] 5.4 `rg -n "^fn |^pub fn |^impl |^struct |^enum |^const " desktop/server/src/lib.rs` 必须为空（lib.rs 只剩 mod + pub use）
- [x] 5.5 `rg -n "SINGLE_SESSION_ID|reject_unknown_session" desktop/server` 必须为空（守卫已彻底移除）

## 6. 文档与归档准备

- [x] 6.1 视情况更新 `desktop/server/server.toml.example` 顶部注释（如提及 `start()` 则改为 `prepare()`；当前文件主要是配置示例，可能无需改）
- [x] 6.2 在 PR 描述中链接本 change 路径 `openspec/changes/desktop-server-decouple-runtime/`，并提示归档后 `openspec/specs/server/spec.md` 将按 delta 重写两条 requirement、新增两条 requirement
- [x] 6.3 运行 `openspec validate desktop-server-decouple-runtime --strict` 通过
