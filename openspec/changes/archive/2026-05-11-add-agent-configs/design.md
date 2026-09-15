## Context

`desktop/server` 当前在启动时从 `~/.nova/server.toml` 解析一组 OpenAI-compatible completion 凭据与能力，并用这组配置构造单个 `ChatSession` 的 harness。`desktop/web` 通过 `ChatClient` 访问 HTTP/SSE API，UI 仍是单 active session 形态，title bar 只有 reset 操作，composer 只负责输入与发送。

本变更把“agent”定义为桌面本地可管理的运行时配置集合。它不是 `nova-core::Agent` 的新公共类型，而是 `desktop/server` 配置域里的实体：一个 agent 通过 id/name 指向 completion 配置，并预留 tools、system prompt、context 等后续扩展位。session 绑定 agent 后，后续 turn 使用该 agent 的配置组装 harness。

## Goals / Non-Goals

**Goals:**
- 在 `~/.nova/server.toml` 中持久化多个 completions 和多个 agents。
- 在独立 session 配置文件中持久化默认 agent 和 session-agent 绑定。
- 提供 server HTTP API，让 web 可以读取 agents、查看当前 session agent、切换当前 session agent，并保存 agent 的 completion 选择。
- 在 web title bar 提供 agents 设置入口，用 card 列出 agents，并在详情卡中用 dropdown 修改 agent 的 completion。
- 在 composer 左下角提供当前 session 的 Agent 选择菜单，切换后 session 记住所选 agent，后续对话使用该 agent。
- 维持现有 Vue 3 + Vite + plain JavaScript + `ChatClient` 边界，不引入 UI 框架。

**Non-Goals:**
- 不在 web 中编辑 completion 的 api key/base URL/model 等详细字段；这些字段仍通过 `server.toml` 或后续专门配置能力维护。
- 不引入多 session 列表/创建/删除 UI；当前 session id 仍可保持 `default`。
- 不扩展 `nova-core::Agent`、`nova_sessions::ChatSession` 的公共 trait 边界来表达 desktop agent 配置。
- 不实现云同步、账号级配置、远程共享 agent 或 public HTTP server 安全模型。
- 不在本变更中实现复杂 tools 配置 UI；agent schema 仅预留 tools 字段，server 初期仍可使用默认工具集合。

## Decisions

- Decision: `AgentConfig` 是 desktop server 的配置实体，不是 core runtime 类型。
  - Rationale: `nova-core` 已通过 trait object 支持运行时选择 completion/toolbox；desktop agent 只是应用层的配置组合，不应让核心 crate 承担桌面配置 schema。
  - Alternatives considered:
    - 在 `nova-core` 新增 `AgentConfig`：拒绝，因为会把 UI/磁盘配置概念泄漏进可复用 runtime。
    - 在 web 本地保存 agent：拒绝，因为后续对话由 server 执行，server 必须是绑定关系的权威来源。

- Decision: `server.toml` 只保存 agent 与 completion 配置，session-agent 绑定保存到独立 session 配置文件。
  - Rationale: completion 是 provider/model 凭据，agent 是运行时组合，session binding 是用户状态。拆开后多个 agent 可以复用一个 completion，也能在后续为 agent 添加 tools、system prompt 或 context 配置；同时 session 状态可以独立于 server runtime 配置演进。
  - `server.toml` proposed shape:
    ```toml
    [[completions]]
    id = "a1b2c3d4"
    name = "Local Qwen"
    provider = "openai-compatible"
    base_url = "http://localhost:11434/v1"
    model = "qwen3"

    [completions.capabilities]
    prefill_supported = false

    [[agents]]
    id = "z9y8x7w6"
    name = "Default"
    completion_id = "a1b2c3d4"
    tools = ["builtin"]
    ```
  - Session config proposed shape (`~/.nova/sessions.toml`):
    ```toml

    [sessions]
    default = "z9y8x7w6"

    # Optional: only when a non-default session id needs another agent.
    # other_session = "abcdef12"
    ```
  - Alternatives considered:
    - 内联 completion 到每个 agent：拒绝，因为会复制密钥引用与模型参数，后续改 completion 需要修改多个 agent。
    - 继续使用单 `[credentials]`：拒绝，因为无法表达多个 agent 与 session 绑定。
    - 把 session-agent binding 写入 transcript：拒绝，因为绑定关系是 session 配置，不是会话事件历史。


- Decision: completion 的 `api_key` 可省略，省略时默认读取 `NOVA_OPENAI_API_KEY`；对省略与 `env:` 形式的密钥采用**实时解析**。
  - Rationale: 大多数本地开发和桌面启动场景都会通过环境变量提供 API key。让 `api_key` 可选可以减少 `server.toml` 样板，也避免鼓励用户把密钥写入配置文件；需要多 key 或非默认环境变量时仍可显式写 `api_key = "env:<NAME>"`，也可写字面量。实时解析指在每次为某次 turn 构造 `OpenAIChatCompletion`（或等价适配器）时再读取环境变量，而不是在加载 `server.toml` 时把 key 固定成进程启动瞬间的快照；这样在长驻 server 进程中修改 export、或启动时尚未设置变量但在首次发消息前已设置时，行为更直观。字面量 `api_key` 仍可在加载阶段校验非空，无需每次读盘外状态。
  - Alternatives considered:
    - 要求每个 completion 都显式写 `api_key = "env:NOVA_OPENAI_API_KEY"`：拒绝，因为这是高频默认值，增加重复配置。
    - 只允许环境变量不允许字面量：拒绝，因为本地测试和特殊部署可能需要明确的字面量配置。
    - 仅在 config load 时解析 `env:`：拒绝，与本决策的实时解析语义不一致。

- Decision: 使用统一 config 类型访问配置，文件拆分只是存储实现细节。
  - Rationale: `server.toml` 和 `sessions.toml` 可以分文件保存，但 `desktop/server` 的业务代码应通过一个聚合的 config 类型（例如 `DesktopConfig` / `ConfigStore`）加载、校验、查询和写回配置。这个类型对外提供 `agents()`、`completions()`、`current_agent_for_session(session_id)`、`set_session_agent(session_id, agent_id)`、`update_agent_completion(agent_id, completion_id)` 等操作；内部再决定读写哪些文件。这样后续迁移到 SQLite、部分配置保留 TOML/部分进入 SQLite，或引入缓存，都不需要重写 HTTP routes 和 harness 组装代码。
  - Alternatives considered:
    - 让各个 route/harness 分别读取 `server.toml` 和 `sessions.toml`：拒绝，因为会把存储布局泄漏到业务层，后续更换存储代价大。
    - 先只写文件级 helper，后续再抽象：拒绝，因为本变更已经同时需要跨文件校验和 session-aware resolver，统一类型是更简单的边界。

- Decision: 所有持久化 id 使用 tinyid 格式：8 位 `[a-z0-9]`。
  - Rationale: 该格式足够短，适合 UI 展示和手写 TOML；明确校验规则能避免 server 接收任意字符串作为配置引用。
  - Alternatives considered:
    - UUID：拒绝，因为过长且对本地配置可读性差。
    - 使用 name 作为 key：拒绝，因为显示名需要可编辑，不能承担稳定引用。

- Decision: 不兼容旧单 completion 配置，直接采用新 schema。
  - Rationale: 该 desktop 配置尚未外发，可以把当前分支上的配置格式视为未稳定实现。直接 breaking replace 能减少双格式解析、隐式默认 id 生成和迁移写回逻辑，让实现保持简单。
  - Alternatives considered:
    - 启动时把旧 `[credentials]` / `[capabilities]` 自动归一化为默认 completion + 默认 agent：拒绝，因为当前没有已发布用户需要兼容。
    - 同时长期支持两套读取格式：拒绝，会增加配置路径复杂度。

- Decision: session-agent 切换由 server 持久化，并允许 active turn 期间切换但只影响后续 turn。
  - Rationale: 当前 `ChatSession::post` 每次都会通过 harness 创建独立的 completion、toolbox 和 `Agent` 实例，已经在执行的 turn 持有自己的运行时对象；更新 session-agent 绑定不会改变该 turn。切换只更新独立 session 配置文件，并从下一次 post 的 `create_completion` / `create_toolbox` 生效，不写入 transcript。
  - Alternatives considered:
    - active turn 期间返回 409 `BUSY`：拒绝，因为当前运行时实例创建模型使切换不会影响已经执行的 turn，拒绝会增加不必要的 UX 限制。
    - 只在 web 内暂存选择直到下一次发送：拒绝，因为刷新或重启会丢失绑定，且 server 不知道 session 的真实配置。

- Decision: 用 session-aware config resolver 解析运行时配置。
  - Rationale: 最简单的 server 侧实现是让统一 config 类型同时加载 agent/completion 配置和独立 session 配置，并提供按 `session_id` 解析的 API：`session_id -> agent_id -> AgentConfig -> completion_id -> CompletionConfig`。创建 `ServerHarness` 时传入 `session_id` 和共享 config store/resolver，harness 需要 completion 时通过 `session_id` 联动找到当前 agent 与 completion。这样无需把 session-agent 逻辑下沉到 `nova-core`，也避免在 UI 或 transcript 中复制绑定状态。
  - Implementation note: 当前 `ChatSession::post` 每次 post 都会调用 harness 创建 completion/toolbox，因此不需要为了 agent switch 重建 `ChatSession`。如果未来 session runtime 改为在创建时固定 completion/toolbox，再重新评估是否需要 idle-only switch 或 session 重建。
  - Alternatives considered:
    - 切换 agent 时强制 reset：拒绝，因为这会把“换配置”变成“丢上下文”。
    - 每个 agent 单独 transcript：拒绝，因为需求是 session 记住 agent，而不是 agent 拥有 session。
    - 在 transcript 中记录 agent switch event：拒绝，因为 session-agent 绑定关系由独立配置文件管理，transcript 只保留会话运行事件。
    - 在 web 侧先解析 agent/completion 再传给 server：拒绝，因为 server 才是运行时配置和密钥解析的权威来源。

- Decision: 写回 `server.toml` 和 session 配置文件时不保留用户注释。
  - Rationale: 初期配置写入优先保证 schema 正确与实现简单；保留注释需要 document-preserving TOML 编辑器，会显著增加复杂度。
  - Alternatives considered:
    - 保留注释并做局部编辑：拒绝，当前 agent/completion/session 结构化更新不需要承担这部分复杂度。

- Decision: Web agents 设置只编辑 agent 的 `completion_id`。
  - Rationale: completion 的凭据和 URL 通常包含敏感信息，当前需求只要求详情卡显示 completion dropdown；先把 UI 聚焦在 agent 与 completion 绑定，不引入密钥编辑和校验 UX。
  - Alternatives considered:
    - 在同一页面编辑 completion 详情：延后，因为密钥存储、env 引用和错误校验需要更完整的设置流程。

- Decision: `ChatClient` 扩展 agents/session-agent 方法，Vue 组件不直接调用 `fetch`。
  - Rationale: 现有 web spec 要求组件只通过 `ChatClient` 访问 chat service；新增 API 应沿用这个边界，避免 UI 组件知道 HTTP route 细节。
  - Alternatives considered:
    - 在设置组件内直接 `fetch`：拒绝，因为会破坏现有 transport-agnostic 边界。

- Decision: 持久化的 session→agent 行若指向**已不存在的 agent id**，在加载聚合配置时 **repair** 为 **`[sessions].default`** 并写回 session 配置文件。
  - Rationale: 用户删除或重命名配置后仍保留旧 id 时，静默用未知 id 运行不可接受；直接报错会打断正常使用。在 `default` 已通过校验的前提下，将无效行改回默认并持久化，与「无单独行的会话回退到 `default`」一致且可重启复现。
  - **`[sessions]`** 扁平表：仅一个 **`default`** 键表示默认 agent；其它 `session_id = tinyid` 仅在该会话与默认不同时写入。会话 id **`default`** 的切换更新 **`default`** 键，不单独占一行。
  - Alternatives considered:
    - 整配置加载失败：拒绝，因为对仅损坏 session 行的用户过于严厉。
    - 仅在内存中改写不写盘：拒绝，因为重启会再次遇到同一无效状态。

- Decision: agents / session-agent 的 HTTP API **保留显式 `session_id`**（路径或等价请求目标），与当前仅使用单个 `default` session 的 web 假设解耦。
  - Rationale: 本地 UI 现阶段仍可写死 `default`，但路由形状一次性支持多 session，后续加 session 列表不必 breaking HTTP。
  - Alternatives considered:
    - 隐式单 session 无 session 参数：拒绝，因为未来扩展成本高。

- Decision: 同一 session、同一 JSONL transcript 上切换 agent **不单独处理**「混合历史上下文」类产品策略；切换已保存后从下一 turn 起用新 agent 即可。
  - Rationale: 本阶段不投入额外 UX 或模型侧缓解；与「不重写 transcript、不 append switch 事件」一致。
  - Alternatives considered:
    - 强制 reset 或按 agent 分 transcript：非本变更目标。

## Risks / Trade-offs

- [同一 session 与 transcript 上换 agent 可能带来模型行为差异] -> **Accepted**：本变更不单独缓解；切换只影响后续 turn，不重写历史。
- [用户本地 TOML 与目标形态不一致时加载失败] -> Mitigation: README 与仓库内示例给出目标 `server.toml` / `sessions.toml` 形态。
- [completion 凭据无法在 web 编辑，用户需要手动改 TOML] -> Mitigation: 本变更只满足 agent-completion 选择；completion 编辑作为后续独立能力。
- [active turn 期间切换 agent 时，当前 turn 仍用旧 harness] -> **Accepted**：与接口语义一致；是否在 UI 上额外提示由实现酌情处理，不作强规范。
- [tinyid 碰撞概率虽低但非零] -> Mitigation: 生成时检查当前 config 内所有 completion/agent id，冲突则重试。

## Implementation Plan

1. 扩展 `desktop/server` 配置模型，只支持新的 `[[completions]]` / `[[agents]]` schema。
2. 增加 tinyid 生成/校验、默认 agent 校验，以及保存新 `server.toml` schema 的写入路径。
3. 增加统一 config 类型/存储层，内部读写 `server.toml` 与独立 session 配置文件，对外提供 agent、completion、session binding 的查询与更新方法。
4. 调整 harness/session 组装：创建 harness 时传入 `session_id` 和统一 config resolver，使每次 post 创建 completion/toolbox 时通过 `session_id` 解析当前绑定 agent；agent 切换从下一次 post 生效。
5. 增加 agents 与 session-agent HTTP routes（请求目标包含显式 `session_id`；本地 web 可先固定为 `default`），并补齐 Rust 单元/HTTP 测试。
6. 扩展 `ChatClient`，增加 agents 与 session agent 方法。
7. 实现 web title bar agents 设置入口、agents card 页面、agent detail completion dropdown、composer Agent 选择菜单。
8. 验证 Rust 编译与 web 构建；必要时更新 README / 示例 `server.toml`。
9. Rollback：回滚新增 routes、web UI 和新配置模型；因为尚未外发，不提供自动降级到旧 `[credentials]` / `[capabilities]` 的迁移逻辑。

## Open Questions

- None.
