## Why

`desktop/server` 目前通过 `ServerOptions` 与 `resolve()` 把 OpenAI 凭据和 transcript 路径混在一起，且凭据主要靠固定环境变量名回退；这既不利于桌面应用分发一份可编辑配置，也无法在 TOML 中安全地引用已有环境变量中的 API key。将配置外置为 TOML、在进程内解析为强类型对象，并把 transcript 路径固定为代码约定，可以简化调用方职责并明确敏感字段的加载语义。

## What Changes

- **BREAKING**：移除通过 `ServerOptions` / `resolve()` 为 completion 提供凭据的现有方式；不再从配置或 `NOVA_DESKTOP_TRANSCRIPT_PATH` 读取 transcript 路径。
- 引入 TOML 配置文件作为桌面 server 的配置来源：磁盘格式分为 **`[credentials]`**（`api_key`、`base_url`、`model` 等）与 **`[capabilities]`**（如 **`prefill_supported`**，可省略，默认 **`false`**），解析后与 `ChatCompletionCapabilities` 对齐，便于将来拆分凭证与能力两块。
- Transcript 文件路径从配置中删除，改为在 `desktop/server` 内硬编码（沿用或等价于当前 `default_transcript_path()` 的语义：优先 `~/.nova/desktop-transcript.jsonl`，否则临时目录）。
- **配置文件路径由 `desktop/server` 约定**，放在 **`~/.nova/`**（与同目录 transcript、日志等一致）；**仅保留无参 `start()`**：内部完成读取 TOML → 解析敏感引用 → 构造运行时对象 → 启动 HTTP（不再提供单独的「仅 resolved 启动」API）。
- `ServerHarness` 构造时 **传入 config 对象**；创建 `OpenAIChatCompletion` 时仅从该对象读取必要参数（不再从分散的 options/env 拼装）。
- API key 支持两种字面量形式：**`env: <name>`**（从环境变量 `<name>` 读取实际 key）与无前缀的 **`<content>`**（字面量即为 key）；trim 与空值错误行为在设计中约定。

## Capabilities

### New Capabilities

（无；行为约束归入既有 `server` 能力，通过增量规格修改。）

### Modified Capabilities

- `server`：更新配置与凭据解析需求——由 TOML 驱动的强类型 config、**无参 `start()`** 内部完成加载与启动（与旧「先 resolved 再起动」方案合并为同一入口）、`ServerHarness` 依赖 config、transcript 路径硬编码、API key 的 `env:` 引用语义；并 **BREAKING** 移除用于 completion 的旧 `ServerOptions` / `NOVA_OPENAI_*` 回退组合（以任务与设计为准）。

## Impact

- **代码**：`desktop/server/src/lib.rs`（`config` 模块、无参 **`start()`**、`ServerHarness`、测试辅助）、可能新增 `desktop/server` 下 config 子模块与示例 TOML；`desktop/client` 改为仅调用 **`start().await`**（内部已加载默认 `~/.nova/` 下固定文件名），一般不再自行拼路径或先 `load_config`。
- **依赖**：`desktop/server` 的 `Cargo.toml` 需增加 TOML 解析相关 crate（如 `toml` / `serde` 已有则复用）。
- **规格**：`openspec/specs/server/spec.md` 的「Completion credentials come from env or options」等条款需被增量规格替换或修订；`start(opts)` 相关场景需与新的启动签名对齐。
