## Why

`desktop` 当前只有一组全局 completion 配置，所有会话都隐式使用同一个运行时组合；这会阻止用户在不同模型、base URL、能力开关和工具组合之间切换，也无法让不同 session 稳定记住自己使用的 agent。引入可管理的 agents 配置并把 session 与 agent 绑定，可以把“运行时配置”从单例设置提升为桌面应用的一等概念，同时为后续多工具集、多 provider 和多 session 做好边界。

## What Changes

- 引入 **agents** 概念：一个 agent 是一组可命名、可编辑、可持久化的运行时配置集合，初期至少包含 `id`、显示名、completion 引用/内联配置、能力配置和预留的 tools 配置位。
- **`~/.moray/server.toml`**：`[[completions]]` 与 `[[agents]]`；每条记录唯一 **tinyid** `id`（8 位 `[a-z0-9]`）。敏感 `api_key` 可省略（默认等价 `env:MORAY_OPENAI_API_KEY`）或写 `env:<NAME>`；对省略与 `env:` 在**每次构造 completion 适配器（按 turn）** 读环境变量，不在仅加载配置时固定快照。
- 引入独立 session 配置文件，用于持久化 session 与 agent 的映射；**`[sessions]`** 扁平表：必填 **`default = "<tinyid>"`**，其它 **`session_id = tinyid`** 行仅在该会话与默认不同时需要。
- 引入统一的 server config 类型/访问层来加载、校验、查询和写回配置；`server.toml`、`sessions.toml` 等文件只是当前存储后端的实现细节，业务代码不直接拼接多个文件的结构。
- 扩展 desktop server HTTP API：提供 agents 的读取/更新入口、按显式 `session_id` 查询与切换 session 当前 agent 的入口（API 形态支持多 session；本地 web 可先只使用 `default`），并保证后续对话使用对应 session 绑定的 agent 配置构造或选择运行时 harness。
- 更新 web title bar：增加 agents 设置入口按钮，点击后打开 agents 配置页面/对话框。
- 新增 agents 配置页面：以 card 方式展示不同 agent；点击 agent card 后打开详情配置卡，初期只展示 completion 选择 dropdown，不在 web 中提供 completion 配置编辑能力。
- 更新 web composer：左下角增加 Agent 选择菜单；用户切换后，当前 session 记住所选 agent，并用该 agent 处理后续对话。
- 保持桌面端 agent 管理为本地配置能力；不引入账号、云同步、远程共享配置或 public server 安全模型。

## Capabilities

### New Capabilities
- `desktop-agents`: desktop agent 配置的持久化模型、HTTP 管理 API、默认 agent 语义，以及 session-agent 绑定行为。

### Modified Capabilities
- `server`：`~/.moray/server.toml` 为 **`[[completions]]` / `[[agents]]`**，并有独立 session 配置文件（**`[sessions].default`** 与可选扁平行）；server 按 session 解析 agent 并驱动后续对话。
- `web`: 增加 agents 设置入口、agents card 列表、agent 详情配置卡，以及 composer 中的 Agent 选择菜单。

## Impact

- **配置与数据**：`~/.moray/server.toml` schema 需要替换为 `completions`、`agents` 或等价结构；独立 session 配置文件在 **`[sessions]`** 下保存 **`default`** 与可选的 **`session_id = tinyid`** 行；若某行指向已删除的 agent，加载时 **repair** 为 `default` 并写回；统一 config 类型负责跨文件加载、校验和更新。
- **Server API**：`desktop/server` 需要新增 config 读写模块、agents/session 配置 DTO、HTTP routes 与测试；agent 切换应持久化为下一次 post 生效，不影响已经执行中的 turn。
- **Runtime composition**：`desktop/server` 的 harness/config 组装需从“启动时单一 completion”调整为“按 session 绑定 agent 解析 completion/tools/capabilities”；初期 tools 可保持默认 `CalcTool`，但数据结构预留扩展。
- **Web UI**：`desktop/web` 需要新增 agents 设置对话框、cards、详情卡、composer selector，并通过 `ChatClient` 访问 server 新增 API；继续保持 plain JavaScript + Vue 3，不引入 UI 框架。
- **边界约束**：`desktop/client` 的 Tauri command 面仍只承担系统/生命周期职责；`moray-core` 的 `Agent` / `Session` trait-object 边界不因本变更扩大。
- **Testing**：Rust 侧覆盖 TOML 解析、tinyid 校验/生成、默认 agent、session-agent 绑定、切换约束与 HTTP API；Web 侧覆盖 agent 列表、详情展示、composer 切换与 ChatClient 调用。
