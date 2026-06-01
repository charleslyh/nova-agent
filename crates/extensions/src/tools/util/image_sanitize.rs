use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// API responses sometimes append a stray `"` (or `%22`) after signed COS query params.
fn sanitize_http_url(url: &str) -> String {
    let mut s = url.trim().to_string();
    loop {
        let before = s.clone();
        s = s.trim_end().to_string();
        if s.ends_with("%22") {
            s.truncate(s.len().saturating_sub(3));
            continue;
        }
        if s.ends_with("%27") {
            s.truncate(s.len().saturating_sub(3));
            continue;
        }
        if s.ends_with('"') || s.ends_with('\'') {
            s.pop();
            continue;
        }
        if s == before {
            break;
        }
    }
    s
}

fn markdown_image_url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"!\[([^\]]*)\]\((https?://[^)]+)\)")
            .expect("markdown image url regex")
    })
}

fn sanitize_markdown_http_urls(text: &str) -> String {
    markdown_image_url_regex()
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let alt = &caps[1];
            let url = sanitize_http_url(&caps[2]);
            format!("![{alt}]({url})")
        })
        .into_owned()
}

fn sanitize_json_urls(value: &mut Value) {
    match value {
        Value::String(s) => {
            if s.starts_with("http://") || s.starts_with("https://") {
                *s = sanitize_http_url(s);
            } else if s.contains("http://") || s.contains("https://") {
                *s = sanitize_markdown_http_urls(s);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_json_urls(item);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                sanitize_json_urls(v);
            }
        }
        _ => {}
    }
}

pub(crate) fn sanitize_image_api_response(body: &str) -> String {
    let trimmed = body.trim();
    if let Ok(mut parsed) = serde_json::from_str::<Value>(trimmed) {
        sanitize_json_urls(&mut parsed);
        return serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| body.to_string());
    }
    if trimmed.contains("http://") || trimmed.contains("https://") {
        return sanitize_markdown_http_urls(trimmed);
    }
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_encoded_quote_from_signed_url() {
        let raw = "https://example.com/a.png?q-url-param-list=%22";
        assert_eq!(
            sanitize_http_url(raw),
            "https://example.com/a.png?q-url-param-list="
        );
    }

    #[test]
    fn sanitizes_markdown_image_in_json_payload() {
        let raw = r#"{"markdown":"![cat](https://example.com/a.png?q=%22)"}"#;
        let mut parsed: Value = serde_json::from_str(raw).unwrap();
        sanitize_json_urls(&mut parsed);
        let md = parsed["markdown"].as_str().unwrap();
        assert!(!md.ends_with("%22)"));
        assert!(md.contains("https://example.com/a.png?q="));
    }
}
