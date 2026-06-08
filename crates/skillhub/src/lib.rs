//! External SkillHub marketplace client: **search** and **download** skill packages to disk.
//!
//! Registering or removing skills in the application catalog is not done here — use
//! `SkillCenter::register_from_dir` / `Sonda::uninstall_skill` in `moray-sonda` after download.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_SEARCH_URL: &str = "https://api.skillhub.cn/api/v1/search";
const DEFAULT_PRIMARY_DOWNLOAD_URL_TEMPLATE: &str =
    "https://api.skillhub.cn/api/v1/download?slug={slug}";
const DEFAULT_DOWNLOAD_URL_TEMPLATE: &str =
    "https://skillhub-1388575217.cos.ap-guangzhou.myqcloud.com/skills/{slug}.zip";
const DEFAULT_INDEX_URL: &str =
    "https://skillhub-1388575217.cos.ap-guangzhou.myqcloud.com/skills.json";

const DEFAULT_SEARCH_LIMIT: u32 = 20;
const DEFAULT_SEARCH_TIMEOUT_SECS: u64 = 6;
const DEFAULT_DOWNLOAD_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Error)]
pub enum SkillHubError {
    #[error("invalid slug: {0}")]
    InvalidSlug(String),

    #[error("skill slug must not be empty")]
    EmptySlug,

    #[error("skill '{slug}' is already downloaded at {path} (use force to overwrite)")]
    AlreadyDownloaded { slug: String, path: PathBuf },

    #[error("no download URL candidates for skill '{0}'")]
    NoDownloadUrls(String),

    #[error("all download attempts failed for skill '{slug}': {detail}")]
    DownloadFailed { slug: String, detail: String },

    #[error("SHA256 mismatch for '{slug}': expected {expected}, got {actual}")]
    Sha256Mismatch {
        slug: String,
        expected: String,
        actual: String,
    },

    #[error("search URL is empty")]
    EmptySearchUrl,

    #[error("skills index URL is empty")]
    EmptyIndexUrl,

    #[error("HTTP {status} from {url}")]
    Http { status: u16, url: String },

    #[error("failed to parse JSON: {0}")]
    Json(String),

    #[error("response is not valid UTF-8")]
    Utf8,

    #[error("downloaded content is not a valid zip archive")]
    InvalidZip,

    #[error("unsafe zip entry: {0}")]
    UnsafeZipEntry(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SkillHubError>;

/// SkillHub client: search and download into a local directory.
#[derive(Debug, Clone)]
pub struct SkillHub {
    search_url: String,
    primary_download_url_template: String,
    download_url_template: String,
    index_url: String,
    download_dir: PathBuf,
    search_limit: u32,
    search_timeout_secs: u64,
    download_timeout_secs: u64,
}

impl SkillHub {
    pub fn new(download_dir: impl Into<PathBuf>) -> Self {
        Self {
            search_url: DEFAULT_SEARCH_URL.to_string(),
            primary_download_url_template: DEFAULT_PRIMARY_DOWNLOAD_URL_TEMPLATE.to_string(),
            download_url_template: DEFAULT_DOWNLOAD_URL_TEMPLATE.to_string(),
            index_url: DEFAULT_INDEX_URL.to_string(),
            download_dir: download_dir.into(),
            search_limit: DEFAULT_SEARCH_LIMIT,
            search_timeout_secs: DEFAULT_SEARCH_TIMEOUT_SECS,
            download_timeout_secs: DEFAULT_DOWNLOAD_TIMEOUT_SECS,
        }
    }

    pub fn download_dir(&self) -> &Path {
        &self.download_dir
    }

    pub async fn search(&self, query: &str) -> Result<SearchResult> {
        search(self, query).await
    }

    pub async fn download(&self, slug: &str, force: bool) -> Result<DownloadResult> {
        download(self, slug, force).await
    }

    pub fn validate_slug(slug: &str) -> Result<()> {
        validate_slug(slug)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHubEntry {
    pub slug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub zip_url: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

impl SkillHubEntry {
    fn search_text(&self) -> String {
        let tags_str = self.tags.join(" ");
        let cats_str = self.categories.join(" ");
        format!(
            "{} {} {} {} {} {} {}",
            self.slug, self.name, self.description, self.summary, self.version, tags_str, cats_str,
        )
        .to_lowercase()
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entries: Vec<SkillHubEntry>,
    pub from_remote: bool,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub downloaded_dir: PathBuf,
    pub files_written: usize,
    pub slug: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct RemoteSearchResponse {
    #[serde(default)]
    results: Vec<RemoteSearchItem>,
}

#[derive(Debug, Deserialize)]
struct RemoteSearchItem {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

impl RemoteSearchItem {
    fn resolved_name(&self) -> Option<String> {
        self.display_name
            .as_ref()
            .or(self.name.as_ref())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct SkillsIndex {
    #[serde(default)]
    skills: Vec<SkillHubEntry>,
}

async fn search(hub: &SkillHub, query: &str) -> Result<SearchResult> {
    let query = query.trim().to_lowercase();

    if !query.is_empty() {
        match remote_search(hub, &query).await {
            Ok(entries) => {
                return Ok(SearchResult {
                    entries,
                    from_remote: true,
                });
            }
            Err(err) => {
                tracing::debug!("remote search failed, falling back to index: {err}");
            }
        }
    }

    let index_entries = fetch_index(hub).await?;

    let entries = if query.is_empty() {
        index_entries
    } else {
        let mut scored: Vec<(usize, SkillHubEntry)> = index_entries
            .into_iter()
            .map(|e| {
                let text = e.search_text();
                let score = text.matches(&query).count();
                (score, e)
            })
            .filter(|(score, _)| *score > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.slug.cmp(&b.1.slug)));
        scored.into_iter().map(|(_, e)| e).collect()
    };

    Ok(SearchResult {
        entries,
        from_remote: false,
    })
}

async fn download(hub: &SkillHub, slug: &str, force: bool) -> Result<DownloadResult> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(SkillHubError::EmptySlug);
    }
    validate_slug(slug)?;

    let target_dir = hub.download_dir.join(slug);
    if target_dir.exists() && !force {
        return Err(SkillHubError::AlreadyDownloaded {
            slug: slug.to_string(),
            path: target_dir,
        });
    }

    std::fs::create_dir_all(hub.download_dir())?;

    let mut download_urls = Vec::new();

    let primary = fill_slug_template(&hub.primary_download_url_template, slug);
    if !primary.is_empty() {
        download_urls.push(primary);
    }

    let fallback = fill_slug_template(&hub.download_url_template, slug);
    if !fallback.is_empty() {
        download_urls.push(fallback);
    }

    if let Ok(index_entries) = fetch_index(hub).await {
        if let Some(entry) = index_entries.iter().find(|e| e.slug == slug) {
            if let Some(ref zip_url) = entry.zip_url {
                let url = zip_url.trim().to_string();
                if !url.is_empty() && !download_urls.contains(&url) {
                    download_urls.push(url);
                }
            }
        }
    }

    if download_urls.is_empty() {
        return Err(SkillHubError::NoDownloadUrls(slug.to_string()));
    }

    let mut last_error: Option<SkillHubError> = None;
    let mut zip_bytes: Option<Vec<u8>> = None;

    for url in &download_urls {
        tracing::info!("downloading skill '{}' from: {}", slug, url);
        match download_bytes(url, hub.download_timeout_secs).await {
            Ok(bytes) => {
                if bytes.starts_with(b"PK\x03\x04") {
                    zip_bytes = Some(bytes);
                    last_error = None;
                    break;
                }
                tracing::warn!(
                    "download from {} returned {} bytes but is not a valid zip",
                    url,
                    bytes.len()
                );
                last_error = Some(SkillHubError::DownloadFailed {
                    slug: slug.to_string(),
                    detail: format!("invalid zip content from {url} ({} bytes)", bytes.len()),
                });
            }
            Err(err) => {
                tracing::warn!("download failed for {}: {err}", url);
                last_error = Some(err);
            }
        }
    }

    let bytes = zip_bytes.ok_or_else(|| {
        last_error.unwrap_or_else(|| SkillHubError::DownloadFailed {
            slug: slug.to_string(),
            detail: "no successful download".into(),
        })
    })?;

    if let Some(expected) = lookup_expected_sha(hub, slug).await {
        let actual = sha256_hex(&bytes);
        if actual != expected {
            return Err(SkillHubError::Sha256Mismatch {
                slug: slug.to_string(),
                expected,
                actual,
            });
        }
    }

    if target_dir.exists() && force {
        std::fs::remove_dir_all(&target_dir)?;
    }

    let files_written = extract_zip_to_dir(&bytes, &target_dir)?;
    let version = read_meta_version(&target_dir).unwrap_or_default();

    Ok(DownloadResult {
        downloaded_dir: target_dir,
        files_written,
        slug: slug.to_string(),
        version,
    })
}

pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(SkillHubError::EmptySlug);
    }
    if slug.contains("..") || slug.contains('/') || slug.contains('\\') {
        return Err(SkillHubError::InvalidSlug(format!(
            "'{slug}' must not contain path separators or '..'"
        )));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(SkillHubError::InvalidSlug(format!(
            "'{slug}': only alphanumeric, '-', '_', '.' allowed"
        )));
    }
    Ok(())
}

fn fill_slug_template(template: &str, slug: &str) -> String {
    let raw = template.trim();
    if raw.is_empty() {
        return String::new();
    }
    if raw.contains("{slug}") {
        raw.replace("{slug}", slug)
    } else {
        raw.to_string()
    }
}

async fn remote_search(hub: &SkillHub, query: &str) -> Result<Vec<SkillHubEntry>> {
    let search_url = hub.search_url.trim();
    if search_url.is_empty() {
        return Err(SkillHubError::EmptySearchUrl);
    }

    let encoded_query = urlencoding::encode(query);
    let full_url = format!(
        "{}?q={}&limit={}",
        search_url, encoded_query, hub.search_limit
    );

    let body = fetch_text(&full_url, hub.search_timeout_secs).await?;

    let response: RemoteSearchResponse = serde_json::from_str(&body)
        .map_err(|e| SkillHubError::Json(format!("remote search response: {e}")))?;

    Ok(response
        .results
        .into_iter()
        .filter(|item| !item.slug.trim().is_empty())
        .map(|item| {
            let slug = item.slug.trim().to_string();
            let name = item
                .resolved_name()
                .unwrap_or_else(|| slug.clone());
            SkillHubEntry {
                slug,
                name,
                description: item.description.unwrap_or_default().trim().to_string(),
                summary: item.summary.unwrap_or_default().trim().to_string(),
                version: item.version.unwrap_or_default().trim().to_string(),
                tags: Vec::new(),
                categories: Vec::new(),
                zip_url: None,
                homepage: None,
                sha256: None,
                source: None,
            }
        })
        .collect())
}

async fn fetch_index(hub: &SkillHub) -> Result<Vec<SkillHubEntry>> {
    let index_url = hub.index_url.trim();
    if index_url.is_empty() {
        return Err(SkillHubError::EmptyIndexUrl);
    }

    let body = fetch_text(index_url, DEFAULT_SEARCH_TIMEOUT_SECS).await?;

    if body.trim_start().starts_with('[') {
        serde_json::from_str(&body).map_err(|e| SkillHubError::Json(format!("skills index array: {e}")))
    } else {
        let index: SkillsIndex = serde_json::from_str(&body)
            .map_err(|e| SkillHubError::Json(format!("skills index object: {e}")))?;
        Ok(index.skills)
    }
}

async fn lookup_expected_sha(hub: &SkillHub, slug: &str) -> Option<String> {
    let entries = fetch_index(hub).await.ok()?;
    entries
        .iter()
        .find(|e| e.slug == slug)
        .and_then(|e| e.sha256.clone())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

fn read_meta_version(skill_dir: &Path) -> Option<String> {
    let meta_path = skill_dir.join("_meta.json");
    if !meta_path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&meta_path).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&raw).ok()?;
    meta.get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_zip_to_dir(bytes: &[u8], target_dir: &Path) -> Result<usize> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|_| SkillHubError::InvalidZip)?;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|_| SkillHubError::InvalidZip)?;
        let entry_path = Path::new(entry.name());
        if entry_path.is_absolute()
            || entry_path
                .components()
                .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(SkillHubError::UnsafeZipEntry(entry.name().to_string()));
        }
    }

    std::fs::create_dir_all(target_dir)?;

    let mut files_written = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| SkillHubError::InvalidZip)?;
        let out_path = target_dir.join(entry.name());

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|_| SkillHubError::InvalidZip)?;
            std::fs::write(&out_path, &buf)?;
            files_written += 1;
        }
    }

    Ok(files_written)
}

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn shared_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("skillhub HTTP client")
    })
}

async fn fetch_bytes(url: &str, timeout_secs: u64) -> Result<Vec<u8>> {
    let response = shared_http_client()
        .get(url)
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(|e| SkillHubError::DownloadFailed {
            slug: String::new(),
            detail: format!("request to {url}: {e}"),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(SkillHubError::Http {
            status: status.as_u16(),
            url: url.to_string(),
        });
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| SkillHubError::DownloadFailed {
            slug: String::new(),
            detail: format!("read body from {url}: {e}"),
        })
}

async fn fetch_text(url: &str, timeout_secs: u64) -> Result<String> {
    let bytes = fetch_bytes(url, timeout_secs).await?;
    String::from_utf8(bytes).map_err(|_| SkillHubError::Utf8)
}

async fn download_bytes(url: &str, timeout_secs: u64) -> Result<Vec<u8>> {
    fetch_bytes(url, timeout_secs).await
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_slug_template_replaces_placeholder() {
        assert_eq!(
            fill_slug_template("https://example.com/skills/{slug}.zip", "calendar"),
            "https://example.com/skills/calendar.zip"
        );
    }

    #[test]
    fn validate_slug_ok() {
        assert!(validate_slug("calendar").is_ok());
        assert!(validate_slug("my-skill").is_ok());
    }

    #[test]
    fn validate_slug_rejects_traversal() {
        assert!(validate_slug("../etc").is_err());
        assert!(validate_slug("foo/bar").is_err());
    }

    #[test]
    fn new_sets_download_dir() {
        let hub = SkillHub::new("./skills");
        assert_eq!(hub.download_dir(), Path::new("./skills"));
    }

    #[test]
    fn remote_search_item_accepts_name_and_display_name() {
        let raw = r#"{
            "slug": "git",
            "name": "Git",
            "displayName": "Git Display",
            "description": "desc",
            "summary": "summary",
            "version": "1.0.0"
        }"#;
        let item: RemoteSearchItem = serde_json::from_str(raw).expect("parse item");
        assert_eq!(item.resolved_name().as_deref(), Some("Git Display"));

        let response: RemoteSearchResponse = serde_json::from_str(
            r#"{"results":[{"slug":"git","name":"Git","displayName":"Git Display"}]}"#,
        )
        .expect("parse response");
        assert_eq!(response.results.len(), 1);
    }
}
