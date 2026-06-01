use std::path::Path;

use moray_core::MorayError;
use uuid::Uuid;

const PRIVATE_UPLOAD_SERVER: &str = "http://21.215.220.113:443/cos_upload";

fn ext_from_magic_bytes(data: &[u8]) -> &'static str {
    if data.len() < 8 {
        return "bin";
    }
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return "jpg";
    }
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "png";
    }
    if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        return "gif";
    }
    if data.len() >= 12
        && data.starts_with(&[0x52, 0x49, 0x46, 0x46])
        && &data[8..12] == b"WEBP"
    {
        return "webp";
    }
    if data.starts_with(&[0x42, 0x4D]) {
        return "bmp";
    }
    "bin"
}

fn mime_from_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

pub(crate) async fn upload_local_file(path: &Path) -> Result<String, MorayError> {
    let data = tokio::fs::read(path).await.map_err(|e| {
        MorayError::Message(format!(
            "upload_local_file: failed to read {}: {e}",
            path.display()
        ))
    })?;

    if data.is_empty() {
        return Err(MorayError::Message(format!(
            "upload_local_file: cannot upload empty file: {}",
            path.display()
        )));
    }

    let ext = ext_from_magic_bytes(&data);
    let filename = format!("{}.{}", Uuid::new_v4(), ext);
    let mime = mime_from_ext(ext);

    let part = reqwest::multipart::Part::bytes(data)
        .file_name(filename)
        .mime_str(mime)
        .map_err(|e| MorayError::Message(format!("upload_local_file: multipart part: {e}")))?;

    let form = reqwest::multipart::Form::new().part("file", part);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| MorayError::Message(format!("upload_local_file: client: {e}")))?;

    let resp = client
        .post(PRIVATE_UPLOAD_SERVER)
        .multipart(form)
        .send()
        .await
        .map_err(|e| MorayError::Message(format!("upload_local_file: request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(MorayError::Message(format!(
            "upload_local_file: HTTP {}: {}",
            status.as_u16(),
            body
        )));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| {
        MorayError::Message(format!("upload_local_file: invalid response JSON: {e}"))
    })?;

    body.get("url")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            MorayError::Message("upload_local_file: response missing 'url' field".into())
        })
}
