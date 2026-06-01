//! Minimal string stubs (moray used fluent `t!` macros).

#[macro_export]
macro_rules! t {
    ("qq-image-upload-mime", url = $url:expr, content_type = $ct:expr) => {
        format!("[用户上传图片是:{} ({})]", $url, $ct)
    };
    ("qq-image-upload", url = $url:expr) => {
        format!("[用户上传图片是:{}]", $url)
    };
    ("qq-emoji", text = $text:expr) => {
        format!("[emoji: {}]", $text)
    };
    ("wecom-approval-title") => {
        "Tool approval".to_string()
    };
    ("wecom-approval-desc", tool = $tool:expr) => {
        format!("Approve tool: {}", $tool)
    };
    ("wecom-sub-title-text", tool = $tool:expr, args = $args:expr) => {
        format!("Tool: {}\n{}", $tool, $args)
    };
    ("wecom-btn-allow") => {
        "Allow".to_string()
    };
    ("wecom-btn-deny") => {
        "Deny".to_string()
    };
    ("wecom-status-allowed") => {
        "Allowed".to_string()
    };
    ("wecom-status-denied") => {
        "Denied".to_string()
    };
    ("wecom-approval-status-title", emoji = $e:expr) => {
        format!("{} Approval", $e)
    };
    ("wecom-btn-allowed") => {
        "Allowed".to_string()
    };
    ("wecom-btn-denied") => {
        "Denied".to_string()
    };
    ("wecom-label-image") => {
        "Image".to_string()
    };
    ("wecom-label-file") => {
        "File".to_string()
    };
    ("wecom-upload-path", label = $label:expr, path = $path:expr) => {
        format!("[{} saved: {}]", $label, $path)
    };
    ("wecom-upload-link", label = $label:expr, url = $url:expr) => {
        format!("[{}: {}]", $label, $url)
    };
    ("wecom-cancelled") => {
        "Cancelled".to_string()
    };
    ("wecom-welcome-default") => {
        "你好！我是 Moray 助手，有什么可以帮你的吗？".to_string()
    };
    ("channel-cmd-new-ok") => {
        "已清空对话记录，可以开始新的对话。".to_string()
    };
    ("channel-cmd-new-failed") => {
        "清空对话失败，请稍后再试。".to_string()
    };
}
