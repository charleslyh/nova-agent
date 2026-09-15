## 1. 依赖与配置模型

- [x] 1.1 在 `desktop/server/Cargo.toml` 中加入 TOML 解析依赖（如 `toml`），与现有 `serde` 对齐 workspace 版本策略
- [x] 1.2 定义磁盘 TOML 模型：**`[credentials]`** / **`[capabilities]`** 两段 serde 结构；`ResolvedServerConfig` 含已解析凭据（`api_key`、`base_url`、`model`）与 **`ChatCompletionCapabilities`**（`prefill_supported` 默认 **`false`**）；导出公开类型与 `ConfigError`

## 2. 加载、解析与硬编码路径

- [x] 2.0 实现 **`config_file_path()`**：默认 **`$HOME/.nova/<固定文件名>.toml`**（文件名在 crate 内常量一处定义），无 `HOME` 时与 transcript 路径的回退策略对齐；并实现 **`load_config()`** / **`load_config_from(path)`**
- [x] 2.1 实现从 `Path` 读取 TOML 并解析 **`[credentials].api_key`**：`trim` 后以 `env:` 前缀则对后缀 `trim` 得到变量名并读环境变量，否则字面量；空名、缺失或空值返回明确错误
- [x] 2.2 实现硬编码 `desktop_transcript_path() -> PathBuf`（保留原 `default_transcript_path` 语义），删除配置与 `NOVA_DESKTOP_TRANSCRIPT_PATH` 相关逻辑
- [x] 2.3 删除 `ServerOptions`、`resolve(&ServerOptions)` 及用于 completion 的 `NOVA_OPENAI_*` 回退常量/逻辑；移除从配置解析 transcript 的字段

## 3. Server 与 Harness

- [x] 3.1 仅提供无参 **`start()`**（内部 `load_config` 再启动）；绑定 listener 前不再使用 `NOVA_OPENAI_*` 回退
- [x] 3.2 调整 `ServerHarness`：构造时持有 config（或字段快照），创建/暴露 `OpenAIChatCompletion` 时使用 **`api_key`、`base_url`、`model` 与 `ChatCompletionCapabilities`**
- [x] 3.3 使用硬编码 transcript 路径创建 `JsonlTranscriptStore` 并确保父目录创建逻辑与现有一致

## 4. 调用方与测试

- [x] 4.1 更新 `desktop/client`：仅调用 **`start().await`**；移除 `ServerOptions::default()` 与「先 load 再 start」，客户端**不再**自行拼接配置路径
- [x] 4.2 更新 `desktop/server` 内集成测试：`HOME` 指向临时目录；在 **`$TEMP_HOME/.nova/`** 写入符合 **`[credentials]` / `[capabilities]`** 形状的 TOML 后调用 **`start()`**；解析单测可继续使用 **`load_config_from`**；删除旧 `test_opts` 形状
- [x] 4.3 （可选）在仓库中增加示例 TOML（含两段表结构），并注明落盘位置为 **`~/.nova/<文件名>.toml`**

## 5. 验证

- [x] 5.1 运行 `cargo test -p nova-desktop-server` 与涉及 `desktop/client` 的构建/测试，确认无 Clippy 警告（若 CI 要求）
