## Context

目标是在 Rust workspace 中实现多 crate Agent 系统：结构设计时应在**多种**经典原则与工程约束之间权衡——例如 **SOLID** 中的职责划分与依赖倒置（SRP、DIP）、对扩展开放/对修改关闭（OCP）、接口隔离（ISP）、可替换性（LSP），以及高内聚低耦合、可测试性、最小惊讶、KISS 等；**文档中若举例 SRP、DIP、OCP，仅作代表，而非穷尽**。LLM **交互协议多样**（chat、文生图等），但 **Agent 主循环只依赖一种「OpenAI chat completions 行为模型」**：消息角色、流式增量、工具调用等语义与 OpenAI Chat Completions / Responses 对齐的抽象；具体厂商 HTTP/SDK 放在适配实现中。用户强调 **状态机可恢复** 与 demo 侧 **概念一致、重复逻辑可复用**。

**Crate 命名**：在 **`Cargo.toml`** 中，可发布 crate 使用 **`moray-*` 包名**（如 **`moray-core`**、未来 `moray-session`），降低 crates.io 撞名风险；仓库内目录可采用短名（如 **`core/`**、**`demo/`**），**不必**与包名一致。**demo** 包名建议仍为 **`moray-demo`** 并保持 **`publish = false`**。

**本阶段**：实现 **`moray-core` + demo**；**不实现 `moray-session`**，会话与转录持久化的产品化封装留待后续变更。

## Goals / Non-Goals

**Goals**

- 用**显式状态机 + 追加式领域事件**表达一次「QA 段」内的 ReAct 生命周期，使 `resume` 与 `invoke` 共享同一套事件消费路径。
- **`CompletionProvider` + `Toolbox`** trait：`CompletionProvider` 覆盖流式 chat completion、恢复所需的续写能力标志；`Toolbox` 覆盖列举与执行工具。
- 提供 `examples/chat`：**真实 `OpenAICompletionProvider`**（`async-openai` 适配，`moray_core::completions::openai`）由 demo 层 **`completion_provider_from_env`** 读取 **`MORAY_OPENAI_*`** 装配；工具侧现阶段仍可用 demo **mock toolbox**（如 echo）演示授权与事件流。本阶段在 demo 内完成转录加载与 QA 分段（或抽到 `moray-core` 的无依赖小模块），验证回放、未完成段恢复、REPL。
- 集成测试：`moray-core` 使用 **`moray-demo::mock`**（dev-dependency）覆盖恢复、工具授权中断、无 **prefill**（`CompletionCapabilities::prefill_supported`）时「丢弃末轮未确认 completion 并重试」；**不依赖**真实云 API。

**Non-Goals（本阶段）**

- **`moray-session` crate**：转录产品化读写、会话级 API（后续变更）。
- **自研**完整 HTTP 客户端替代成熟生态：OpenAI 兼容路径实现 **`CompletionProvider` 适配器时，优先依赖开源 crate [`async-openai`](https://crates.io/crates/async-openai)**，在 trait 边界内封装，避免重复造轮子；Anthropic 等厂商同理优先选用成熟异步客户端（具体 crate 选型在实现任务中确定）。
- 在 **`CompletionProvider` trait 中**直接承载文生图等非 chat 协议（应单独 trait/流水线，需要时再转为 chat 消息由适配层承接）。
- 多模态、复杂并行工具调用、子 Agent。
- 强 schema 的跨厂商统一 function-calling（先以工具名 + JSON 参数 + 结果字符串为核心抽象）。

## Crate 划分（当前 vs 后续）

| 目录（示例） | `Cargo.toml` 包名 | 职责 | 本阶段 |
|--------|------|------|--------|
| `core/` | **`moray-core`** | 类型、状态机、`CompletionProvider`/`Toolbox` trait、ReAct 引擎、`Event`、恢复纯逻辑 | 实现 |
| （预留） | `moray-session` | 转录序列化/反序列化、QA 分段、会话级 API | **不实现** |
| `demo/` | **`moray-demo`**（`publish = false`） | `examples/*`、CLI 薄 UI、集成测试 | 实现 |

依赖方向：**`moray-demo` → `moray-core` only**；**禁止** `moray-core` 依赖 demo。未来引入 `moray-session` 后为 **`moray-demo` → `moray-session` → `moray-core`**。

### 动态多态（`CompletionProvider` / `Toolbox`）

- **模型**与**工具箱**在集成边界上应按 **trait object（`dyn Trait`）** 使用，例如 `Box<dyn CompletionProvider + Send + Sync>`、`Arc<dyn …>`，以便运行时切换具体实现而 **不必** 把 `Agent` 做成新的泛型类型。
- `moray-core` 为 `Box` / `Arc` 提供对 `CompletionProvider`、`Toolbox` 的 **转发实现**，`Agent::new` 可直接持有上述智能指针。
- 单元 / 集成测试仍可用具体 mock 类型；示例 `chat` 演示 `Box<dyn …>` 接线。

## 核心设计决策

### 1. `CompletionProvider` 与「OpenAI 行为兼容」

- **主循环**只面向 **chat completion 抽象**：输入为 chat 消息历史（及工具定义），输出为流式 assistant 片段与工具调用请求，语义与 OpenAI 兼容，便于单测 mock 与多厂商适配。
- **OpenAI 兼容的具体实现**：在 **`moray-core` 的 `providers/openai`** 中基于 [`async-openai`](https://crates.io/crates/async-openai) 实现 **`OpenAICompletionProvider`**，映射到 `CompletionProvider`（chunk：`TextDelta`、逐项 **`ToolCall`**、**`Finished`**）；**不在本仓库重复实现**与 OpenAI HTTP/流协议等价的底层客户端。`api_key`、`api_base`、`model` 由调用方显式传入（无 core 内默认模型或 URL）。
- **文生图等**：属于不同协议；若要与 Agent 结合，应通过**独立客户端**生成结果，再作为**用户/工具消息**写回 chat 历史，而不是扩张 `CompletionProvider` 承担图像扩散的流式原语。

### 2. 领域事件（Event Sourcing 风格）而非仅「messages」

- **原因**：可恢复性要求区分 completion 增量、工具调用请求（待授权）、工具结果等。
- **做法**：对外 **结构化 `Event`**；持久化与状态回放基于同一事件序列。

### 3. 显式状态机

- 示例状态：`Idle` → `RunningCompletion` → `AwaitingToolDecision` → `RunningTool` → … → `SectionComplete` / `Failed`。
- **转移**由：用户输入、completion 流、工具结果、授权回调、持久化事件回放驱动。

### 4. Chat completion「可续写」能力（Capability）

- `CompletionCapabilities::prefill_supported` 暴露是否支持 assistant prefill / continue-final-assistant-message 等等价语义。
- **规则**：**不支持**且进程在 `RunningCompletion` 异常终止时，恢复时 **丢弃该轮未提交的 assistant 增量**，从上一稳定检查点 **重新发起 completion**。

### 5. `invoke` 与 `resume` 的 API 形状

- 二者返回 **同类异步事件流**。
- 集成方直接消费 `invoke` / `resume` 的事件流（例如 demo 内联 `match`）；若多处需要相同逻辑，再在应用或未来 `moray-session` 层提炼共享驱动，而非当前 core 必选 API。

### 6. Toolbox / 授权

- 工具调用请求以事件暴露； harness 持久化决策后继续。

### 7. QA 段与转录（本阶段）

- **QA 段**：一次用户 query 到段完成。
- **本阶段**：分段与文件加载在 **demo**（或 core 内极薄辅助）完成；未来迁移至 `moray-session`。
- **持久化载体（如 `jsonl`）**：初版可采用 **`jsonl` 等简单格式**，**归属 demo 层实现细节**；未来由 **`moray-session` 策略化**（可插拔编码、路径、版本迁移），**不是当前 core 架构的关键决策点**。

### 8. 异步运行时

- 全 workspace **统一使用 `tokio`** 作为异步运行时，以便与 `async-openai` 等生态一致并降低组合复杂度。

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| 事件模式演进 | 转录 `schema_version`；兼容或迁移层 |
| completion 解析与状态机耦合 | 「解析流 → 转移」单独模块 |
| 集成测试脆弱 | 全程 mock；确定性序列 |

## 已决事项（原 Open Questions）

- **事件持久化格式**：**`jsonl` 初版可接受**；当前为 **demo 层**选型，未来 **`moray-session`** 再策略化；**不列为 core 关键设计前提**。
- **异步运行时**：**统一 `tokio`**。
