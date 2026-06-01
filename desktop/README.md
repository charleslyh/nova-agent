# Moray Desktop

Rust 包与核心库同属仓库根 **Cargo workspace**；在仓库根执行 `cargo build --workspace` / `cargo test --workspace` 即可覆盖 desktop 与 `crates/*`。

## Packages

- `server`: `cargo test -p moray-desktop-server` / `cargo build -p moray-desktop-server`
- `cli` (`moray-cli`, at `desktop/cli`): sidecar for skills that run `$CLI tool run …` — `cargo build -p moray-cli`
- `client/src-tauri`: `cargo build -p moray-desktop-client`
- `web`: `pnpm --dir desktop/web install && pnpm --dir desktop/web dev`
- `crates/sonda`: `cargo test -p moray-sonda`

## 本地设置（`settings.toml` / `sessions.toml`）

以下描述的是 **多 completion / 多 agent** 与 `sessions.toml` 的约定。

持久化目录由 **Tauri client** 通过 `AppHandle::path().app_data_dir()` 解析（`com.moray.desktop` 沙盒路径，因平台而异），再传入 embedded server 的 `bootstrap`。典型位置示例：macOS `~/Library/Application Support/com.moray.desktop/`。

| 文件 | 作用 |
|------|------|
| `<app_data>/settings.toml` | `[[completions]]` 与 `[[agents]]`；completion / agent 的 `id` 为 **tinyid**：8 位 `[a-z0-9]`，各自集合内唯一。 |
| `<app_data>/sessions.toml` | 文件根级 **`default_agent_id`**（默认 agent）；可选 **`[[entries]]`** 每行一个 **`session_id`**，可写 **`agent_id`** 覆盖默认；省略 `agent_id` 时仍用 `default_agent_id`。写回时会省略与 `default_agent_id` 相同的 `agent_id`。通过 HTTP 切换某会话的 agent 前，该 **`session_id`** 须已有一条 `[[entries]]` 行；新会话由 **`POST /sessions`** 创建（catalog 行 + 磁盘 transcript）。 |

**`settings.toml`**：当前仅支持含 **`[[completions]]`** 与 **`[[agents]]`** 的形态；含顶层 **`[credentials]`** / **`[capabilities]`** 且无上述数组的写法不被支持。

**`api_key`**

- **字面量**：非 `env:` 前缀的字符串在 **加载配置** 时须非空。
- **省略**：等价于 `env:MORAY_OPENAI_API_KEY`，在 **每次构造 completion 适配器（按 turn）** 时读环境变量；启动时尚未设置变量、在首次发消息前再 export，亦可成功。
- **`env:NAME`**：同样在 **按 turn** 构造适配器时解析；变量名为空或未设置/trim 后为空 → **该次请求** 确定性失败（不要求在仅加载 TOML 时失败）。

**Session 映射**：若某条 `[[entries]]` 的 `agent_id` 已不存在于 `[[agents]]`，加载会失败（须修正配置）。Server 通过 HTTP 管理 agent 与 session 映射时使用显式 **`session_id`**；切换 agent 会更新对应 entry 的 **`agent_id`**（与 `default_agent_id` 相同时写回可省略该字段）。

**对话控制**：`POST /sessions/{session_id}/submit` 提交一轮输入；运行中可用 `POST /sessions/{session_id}/cancel` 停止当前 Agent run（幂等）。`POST /sessions/{session_id}/reset` 在 session busy 时会先 cancel 再清空 transcript。

**写回**：程序写回 `settings.toml` / session 配置时 **不保留** 用户注释。

**示例文件**（仓库内，可复制到 app data 目录）：

- `desktop/server/settings.toml.example` → `<app_data>/settings.toml`
- `desktop/server/sessions.toml.example` → `<app_data>/sessions.toml`

## `moray-cli` sidecar（skills / shell）

Desktop **client 不构建** `moray-cli`。Tauri 通过 `bundle.externalBin` 打包 `src-tauri/resources/binaries/moray-cli-$TARGET_TRIPLE`（见 [`resources/binaries/README.md`](client/src-tauri/resources/binaries/README.md)）。

在 `main.rs` 的 `setup` 中（与 `window-layout.json` 路径决议同级），`bundle::resolve_server_bundle` 一次性解析 sidecar、skills、`settings.toml`、`sessions.toml`、`sessions/` 目录与 `tools.toml` 等路径，并组装 shell 子进程环境变量（`CLI`、`MORAY_TOOLS_CATALOG_PATH`），再传给 `SondaGateway::spawn`。Dev 构建后 sidecar 位于 `target/debug/moray-cli`（由 `tauri-build` 从 `src-tauri/resources/binaries/moray-cli-<triple>` 复制）。

维护者流程（sidecar 必须用 **release** 二进制，见 `resources/binaries/README.md`）：

```bash
cargo build -p moray-cli --release
# 按 resources/binaries/README.md 从 target/release/moray-cli 复制到 src-tauri/resources/binaries/moray-cli-<triple>
cargo build -p moray-desktop-client   # 或 cargo tauri dev
```

直接试跑 CLI：

```bash
./target/release/moray-cli tool run web_fetch --args '{"url":"https://example.com"}'
```

## Agent skills（client 打包）

内置 skills、tools manifest、CLI sidecar 由 **Tauri client** 统一管理在 `desktop/client/src-tauri/resources/`（源码：`skills/`、`tools.toml`、`binaries/`）。`tauri.conf.json` 将 `resources/skills/` → bundle 根目录 `skills/`、`resources/tools.toml` → `tools.toml`，`externalBin` 将 `moray-cli-<triple>` 复制为 bundle 根目录 `moray-cli`。路径解析在 `src-tauri/src/bundle.rs`；server 仅从解析得到的目录加载到 `SkillCenter`。

新增 skill：在 `src-tauri/resources/skills/<name>/` 下添加 `SKILL.md` 或 `SKILL.toml`，重建 client 即可。

## Verification

1. 仓库根：`cargo build --workspace && cargo test --workspace`
2. Web build: `pnpm --dir desktop/web install && pnpm --dir desktop/web build`
3. End-to-end（本地）：在 app data 目录写好 `settings.toml` 与 `sessions.toml`；示例见 `desktop/server/settings.toml.example` 与 `desktop/server/sessions.toml.example`。启动 web dev server 后在仓库根执行 `just tauri-dev` 或 `cargo tauri dev`（在 `desktop/client` 目录）
