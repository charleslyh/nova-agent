> Historical note (2026-04-29): task items mentioning `openai` feature in core are preserved as-is for archival traceability; current code moved concrete OpenAI adapter to `nova-builtin` (`nova_builtin::completions::OpenAIChatCompletion`).

## 1. 仓库与工作区

- [x] 1.1 在仓库根建立 Rust workspace（根 `Cargo.toml`），加入 **`core/`** 与 **`demo/`**（包名分别为 **`nova-core`**、**`nova-demo`**；`nova-demo` 设置 **`publish = false`**）。**本阶段不添加 `nova-session` 成员。**
- [x] 1.2 约定 Rust edition、最低工具链（MSRV）与 `rustfmt`/`clippy` 检查方式（CI 可选）；异步运行时 **统一 `tokio`**。

## 2. 包 `nova-core`（目录 `core/`）

- [x] 2.1 定义核心类型：`Event`（或可扩展 enum）、错误类型、（可选）QA「段」边界在 core 中的表达。
- [x] 2.2 定义 **`CompletionProvider`** trait：OpenAI 行为兼容的 chat completion（流式输出、`CompletionChunk`、消息/工具形状与恢复相关能力）；**`CompletionCapabilities::prefill_supported`** 等。
- [x] 2.3 定义 **`Toolbox`** trait：列出工具、执行调用（mock 返回确定性结果）。
- [x] 2.4 实现显式状态机 + ReAct 循环：`invoke(user_text)` 与 `resume(prior_events)` 汇入同一内部路径。
- [x] 2.5 实现恢复规则：工具授权挂起可续；无 prefill 能力时 completion 轮次异常中断后丢弃未确认输出并重试（含单元测试）。
- [x] 2.6 抽出「消费事件流」的共享辅助（可供 demo/tests 使用）。

## 3. nova-session（后续变更，本阶段跳过）

- 转录读写、QA 分段、会话级 API 留待引入 **`nova-session`** crate 的独立 OpenSpec 变更；本 tasks 不执行。

## 4. Demo 与验证

- [x] 4.1 **本阶段**在 **`demo/`** 内实现转录加载与 QA 分段（或可抽至包 **`nova-core`** 的无 IO 纯函数 + demo 负责读写文件），满足 `chat` 流程需求；持久化格式（如 **`jsonl`**）为 **demo 层细节**，未来由 `nova-session` 策略化。
- [x] 4.2 实现 `examples/chat`：加载转录 → 回放已完成 QA → **`OpenAICompletionProvider`**（`completion_provider_from_env` / **`NOVA_OPENAI_*`**）+ demo mock toolbox → `resume` pending → REPL `invoke`；**共用**事件流驱动器。
- [x] 4.3 集成测试：**`nova-demo::mock`**（`CompletionProvider` + toolbox），覆盖 resume、授权中断后恢复、无 prefill 时的重试策略。
- [x] 4.4 feature **`openai`**：**`OpenAICompletionProvider`**（`completions/openai.rs`）— **[`async-openai`](https://crates.io/crates/async-openai)**；调用方提供 key、base URL、model（demo 经 **`completion_provider_from_env`** 读环境变量）。

## 5. 规格与收尾

- [x] 5.1 本变更通过 `openspec validate add-multi-crate-agent-system --strict`。
- [x] 5.2 更新 `openspec/project.md` 中 Purpose / Tech Stack / Testing（与实现一致）。
