## Why

项目需要一套可扩展、可测试的多 crate Agent 运行时：结构设计时综合考量职责边界、依赖方向、扩展点等（见 `design.md`；**不限于** SOLID 中的个别条目），使核心与具体 LLM **传输/协议**解耦，主循环只依赖 **OpenAI 行为兼容的 chat completion** 抽象；支持 ReAct 式循环与**可恢复状态机**（崩溃或中断后基于事件转录恢复），并在 demo 与集成测试中用 mock 验证设计，而非一开始就依赖真实云 API。

## What Changes

- 以 **Cargo workspace** 组织 crate：**`moray-core`**（领域与运行时）、以及**默认不发布**的 **demo crate**（可运行示例与集成测试；名称不必服从 crates.io 全局唯一约束，例如 `moray-demo` + `publish = false`）。**本阶段不实现 `moray-session`**（命名保留给后续会话/转录 harness）。
- **Moray Core**：**`CompletionProvider`**（或等价命名）trait — 表示主循环所需的 **OpenAI 兼容 chat completion**（流式消息、工具调用形状等与该协议对齐）；**`Toolbox`** trait、统一 **领域事件（Event）**、**显式状态机**、**ReAct 循环**；对外 `invoke` 与 `resume` 产出**同一套事件流抽象**。其它形态（文生图等非 chat-completion 协议）不进入主循环 trait，可由独立适配层封装后再接入 chat 消息。
- **可恢复性**（与此前一致）：
  - 工具调用待授权等挂起点：**仅**通过持久化事件 + 状态机恢复继续；
  - 若进程在**非「chat completion 可续写」**场景下中断，且实现**不支持** prefill / assistant 续写：须 **丢弃该轮未确认的 assistant 输出**，从上一稳定检查点 **整轮重新请求 completion**。
- **本阶段 Demo**：`examples/chat` — load transcript → 回放已完成 QA → **`OpenAICompletionProvider`**（demo 层 `completion_provider_from_env` / `MORAY_OPENAI_*`）+ mock toolbox → `resume` pending → REPL；转录加载与 QA 分段**由 demo（或 core 内极小纯函数）完成**；**共享「消费事件流」逻辑**。集成测试仍全程 mock（`moray-demo`），不依赖云 API。

## Impact

- 受影响 specs（本变更新增 delta）：`moray-core`、`moray-demos`。（`moray-session` 仅在 design 中预留，无本变更 spec delta。）
- 受影响代码：workspace 成员目录可为 `core/`、`demo/` 等短路径；**包名**仍为 **`moray-core`**、**`moray-demo`**（见各 crate 的 `Cargo.toml`）。

## Notes

- `openspec/project.md` 当前为模板占位；实现阶段应同步补全技术栈（Rust edition、MSRV、fmt/clippy）与测试策略，以便与本提案一致。
