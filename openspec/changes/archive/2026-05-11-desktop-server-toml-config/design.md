## Context

`desktop/server` 的 `config` 内联模块提供 `ServerOptions` 与 `resolve()`：凭据字段可选，缺失时回退到 `MORAY_OPENAI_*` 环境变量；transcript 路径可由选项、`MORAY_DESKTOP_TRANSCRIPT_PATH` 或默认 `~/.moray/desktop-transcript.jsonl` 决定。`start(opts)` 在内部完成 resolve 并构造 `OpenAIChatCompletion` 与 `JsonlTranscriptStore`，`ServerHarness` 仅持有已构造的 `completion` 克隆。`desktop/client` 以 `ServerOptions::default()` 调用 `start`，完全依赖环境变量。

## Goals / Non-Goals

**Goals:**

- 用 **TOML 文件** 作为 completion 相关设置的唯一来源（在「已解析为内存对象」这一层不再依赖 `MORAY_OPENAI_*` 作为回退）。
- 在 **HTTP 监听启动之前** 完成：读文件 → 反序列化 → 解析 `api_key` 的 `env:` 引用 → 得到 **resolved config**。
- **`ServerHarness` 持有 resolved config**，在构造/提供 `OpenAIChatCompletion` 时从该对象读取 **凭证**（`api_key` / `base_url` / `model`）与 **`ChatCompletionCapabilities`**（来自 `[capabilities]`，未写则默认）。
- **Transcript 路径** 从任何用户配置中移除，在库内 **单一硬编码函数**（语义与当前 `default_transcript_path()` 一致：`$HOME/.moray/desktop-transcript.jsonl`，无 `HOME` 时用系统 temp + 固定文件名）。
- **`api_key` 字段**：若 TOML 字符串在去除首尾空白后以 **`env:`** 开头（大小写敏感），则去掉该前缀并对剩余部分 **trim** 得到环境变量名，从 `std::env::var` 读取真实 key；否则整段字符串（trim 后）视为字面量 key。解析时 **不得** 把字面量误当作 `env:`（仅以前缀判定）。
- **配置文件路径由 `desktop/server` 自行约定**：与用户目录下其它 Moray 数据一致，放在 **`$HOME/.moray/`**（与当前 transcript、日志等产物同级；无 `HOME` 时的回退策略与 transcript 硬编码函数对齐）。
- TOML 在结构上分为 **`[credentials]`** 与 **`[capabilities]`** 两块：`credentials` 含 provider 密钥与端点；`capabilities` 映射到 `moray_core::ChatCompletionCapabilities`（当前至少 **`prefill_supported`**，**可省略**，省略时视为 **`false`**），便于日后把两块分别抽离或复用。
- 为 `desktop/client` 提供 **单一入口**：公开异步 **`start()`**（无参数），在内部按默认路径完成 **读 TOML → 解析 → 构造 `ResolvedServerConfig` → 起 HTTP**；不再提供单独的「先 resolved 再起动」API。测试或非默认路径需求通过 **`load_config_from`** 做解析校验，或通过设置 **`HOME`** / 在默认路径下放置 **`server.toml`** 配合 **`start()`**。

**Non-Goals:**

- 不为 `base_url` / `model` 引入 `env:` 前缀（用户仅要求 api key）。
- 不引入加密配置文件或密钥链集成。
- 不改变除 completion 凭据与 transcript 路径来源以外的 HTTP 行为或单会话语义。

## Decisions

1. **配置分层：`FileConfig`（Serde TOML）与 `ResolvedServerConfig`（可启动）**  
   - 磁盘 TOML 使用两个顶层表：**`[credentials]`**（`api_key`、`base_url`、`model` 等）与 **`[capabilities]`**（`prefill_supported` 等，与 `ChatCompletionCapabilities` 对齐）；`capabilities` 整表可省略，省略时 **`ChatCompletionCapabilities::default()`**（`prefill_supported == false`）。  
   - `ResolvedServerConfig`（名称以实现为准）至少包含：已解析的凭据（`api_key` 已解开 `env:`）、**独立的** `capabilities: ChatCompletionCapabilities`，供 `OpenAIChatCompletion::new(..., capabilities)` 使用。  
   - **理由**：凭证与能力分块，便于将来快速拆分或单独注入；解析逻辑可用 **`load_config_from`** 在单元测试中覆盖，集成测试通过 **`HOME` + 默认路径 `server.toml`** 驱动 **`start()`**。

2. **默认配置文件路径由 server 定义；主启动路径无参**  
   - 公开 **`config_file_path() -> PathBuf`**（或等价名称）：在存在 `HOME` 时为 **`PathBuf::from(home).join(".moray").join("<filename>.toml")`**，文件名在设计实现时固定一处（例如 `server.toml`，与 `desktop-transcript.jsonl` 同目录）。无 `HOME` 时与 transcript 使用同一套回退策略（如临时目录下的 `.moray` 或等价路径），保证测试可隔离。  
   - 公开 **`load_config()` / `load_config_from(path)`**：供校验、CLI、单元测试等**仅解析**场景复用；**常规桌面启动不必先调用**（由 **`start()`** 内部加载）。  
   - 公开异步 **`start()`**：无参，内部依次调用默认路径的加载与 HTTP 启动；失败时返回与实现一致的错误类型（如 `Result<ServerHandle, String>`）。  
   - 移除 `start(ServerOptions)` 与 `resolve(&ServerOptions)`；**解析与启动合并为单一无参 `start()`**（内部完成默认路径加载）。  
   - **理由**：路径与加载闭环在 server 内，客户端一行 `start().await`；需自定义路径时仅用 **`load_config_from`** 做离线解析，或调整进程环境与默认路径文件以驱动 **`start()`**。

3. **`ServerHarness` 字段**  
   - 保存 `Arc<ResolvedServerConfig>`（或凭据 + `ChatCompletionCapabilities` 的快照），在 `create_completion` 中基于 **凭证与 capabilities** 构造 `OpenAIChatCompletion::new(api_key, base_url, model, capabilities)`（或与现状一致的预构造 Arc 克隆）。  
   - **理由**：completion 行为同时受凭证与能力位控制；与 TOML 两块结构一致。

4. **Transcript**  
   - 非公开函数 `desktop_transcript_path() -> PathBuf` 实现硬编码规则；`start` / `ServerHarness` 与测试文档化：**测试** 通过设置 **`HOME`** 指向临时目录，使 JSONL 落在临时树内，避免污染真实 `~/.moray`。  
   - **理由**：不在配置中暴露路径的前提下保持可测性。

5. **依赖**  
   - 使用 workspace 已用的 `serde` + **`toml`** crate 反序列化（与 Rust 生态一致）。  
   - **备选**：`figment`——更重，非目标。

6. **`env:` 解析细节**  
   - 前缀检测：`trim_start()` 后以 `env:` 开头；去掉 `env:` 后对名称 `trim()`；名称为空 → 配置错误。  
   - 环境变量值 `trim` 后为空 → 错误（与当前 Empty 语义一致）。  
   - **不** 支持 `ENV:` 大写前缀，避免与字面量密钥偶然冲突（若需可后续 ADDED）。

## Risks / Trade-offs

- **[Risk] 破坏性 API** → 调用方必须同时改 TOML 与启动代码；在 `tasks.md` 中列出 `desktop/client` 与所有 `start` 调用点。  
- **[Risk] 本地开发忘记配置文件** → `client` 需在开发文档或示例中提供样例 `*.toml`（非本变更强制交付物时可记在 tasks 为可选）。  
- **[Trade-off] 仅 api key 支持 `env:`** → 其他字段若未来要敏感化需另一次变更。

## Migration Plan

1. 在仓库中新增示例 TOML（可选），形状包含 `[credentials]` / `[capabilities]`，置于文档或 `~/.moray/` 说明旁。  
2. 实现 `config_file_path`、`load_config()`、`load_config_from(path)`、硬编码 transcript 与 `env:` 解析。  
3. 将主入口改为无参 **`start()`**（内部加载默认 TOML 再启动）；保留 **`load_config` / `load_config_from`** 供解析与校验；删除 `ServerOptions` / `resolve` 的 completion 路径。  
4. 更新 `desktop/client`：仅调用 **`start().await`**；**不再**先 `load_config` 再 `start`，**不再**在客户端拼接 `~/.moray/` 路径。  
5. 更新 `desktop/server` 集成测试：`HOME` 指向 temp，在临时 `HOME/.moray/` 下写入 TOML 后调用 **`start()`**；对 TOML 形状的单元测试可继续使用 **`load_config_from`**。
