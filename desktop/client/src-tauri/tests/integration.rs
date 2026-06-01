//! Desktop [`build_sonda`] integration tests — exercise [`Sonda`] directly, not HTTP.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;

use moray_desktop_client::sonda::{
    build_sonda, materialize_tools_catalog, ensure_sessions_dir, ensure_user_skills_dir,
    SondaRuntimePaths, CHANNELS_CATALOG_FILE_NAME, SESSIONS_CATALOG_FILE_NAME,
    SESSIONS_DIR_NAME, SETTINGS_FILE_NAME, TOOLS_CATALOG_FILE_NAME,
};
use moray_sonda::{SondaSessionTranscripts, SondaStateEvent};
use tokio::time::{timeout, Duration};

const TEST_SESSION_ID: &str = "other";

static HOME_LOCK: Mutex<()> = Mutex::new(());

struct TestHomeGuard {
    _lock: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    previous: Option<String>,
}

impl TestHomeGuard {
    fn new() -> Self {
        let lock = HOME_LOCK.lock().expect("home lock");
        let previous = std::env::var("HOME").ok();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        Self {
            _lock: lock,
            _temp: temp,
            previous,
        }
    }
}

impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn minimal_server_toml() -> &'static str {
    r#"[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "test-model"
api_key = "test-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"
"#
}

fn two_agent_server_toml() -> &'static str {
    r#"[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "test-model"
api_key = "test-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"

[[agents]]
id = "b2c3d4e5"
name = "Alt"
completion_id = "a1b2c3d4"
"#
}

fn minimal_sessions_toml(default_agent_id: &str) -> String {
    format!(
        r#"default_agent_id = "{default_agent_id}"

[[entries]]
session_id = "other"

[[entries]]
session_id = "side"
"#
    )
}

fn test_data_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME set by TestHomeGuard");
    PathBuf::from(home).join("data")
}

fn test_sessions_dir() -> PathBuf {
    test_data_dir().join(SESSIONS_DIR_NAME)
}

fn bundled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/skills")
}

fn bundled_tools_catalog() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/tools.toml")
}

fn bundled_settings() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/settings.toml")
}

fn test_moray_cli_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME set by TestHomeGuard");
    let path = PathBuf::from(home).join("moray-cli");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create moray-cli parent");
    }
    std::fs::write(&path, b"").expect("write stub moray-cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod moray-cli");
    }
    path
}

fn test_runtime_paths() -> SondaRuntimePaths {
    let data_dir = test_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let bundled_skills = bundled_skills_dir();
    let skills_dir_user = ensure_user_skills_dir(&data_dir.join("skills"), &bundled_skills)
        .expect("user skills dir");
    let tools_catalog_path = materialize_tools_catalog(
        &data_dir.join(TOOLS_CATALOG_FILE_NAME),
        &bundled_tools_catalog(),
    )
    .expect("tools catalog");
    let sessions_dir = ensure_sessions_dir(&data_dir).expect("sessions dir");

    SondaRuntimePaths {
        skills_dir_bundled: bundled_skills,
        skills_dir_user,
        settings_path_bundled: bundled_settings(),
        settings_path_user: data_dir.join(SETTINGS_FILE_NAME),
        sessions_catalog_path: data_dir.join(SESSIONS_CATALOG_FILE_NAME),
        channels_catalog_path: data_dir.join(CHANNELS_CATALOG_FILE_NAME),
        sessions_dir,
        tools_catalog_path,
        cli_path: test_moray_cli_path(),
    }
}

fn ensure_session_transcript(session_id: &str) {
    let session_transcripts = SondaSessionTranscripts::new(test_sessions_dir());
    if !session_transcripts.exists(session_id) {
        session_transcripts
            .create(session_id)
            .expect("create test session transcript");
    }
}

fn write_default_settings_files() {
    let data_dir = test_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::write(data_dir.join(SETTINGS_FILE_NAME), minimal_server_toml())
        .expect("write settings.toml");
    std::fs::write(
        data_dir.join(SESSIONS_CATALOG_FILE_NAME),
        minimal_sessions_toml("z9y8x7w6"),
    )
    .expect("write sessions.toml");
    let _paths = test_runtime_paths();
    ensure_session_transcript(TEST_SESSION_ID);
}

fn write_settings_files(server_toml: &str) {
    let data_dir = test_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::write(data_dir.join(SETTINGS_FILE_NAME), server_toml).expect("write settings.toml");
    std::fs::write(
        data_dir.join(SESSIONS_CATALOG_FILE_NAME),
        minimal_sessions_toml("z9y8x7w6"),
    )
    .expect("write sessions.toml");
    let _paths = test_runtime_paths();
}

async fn build_test_sonda() -> moray_sonda::Sonda {
    write_default_settings_files();
    build_sonda(&test_runtime_paths()).expect("sonda should build")
}

async fn build_test_sonda_with_server_toml(server_toml: &str) -> moray_sonda::Sonda {
    write_settings_files(server_toml);
    build_sonda(&test_runtime_paths()).expect("sonda should build")
}

fn session_transcript_path(session_id: &str) -> PathBuf {
    test_sessions_dir()
        .join(session_id)
        .join("transcript.jsonl")
}

#[test]
fn bundled_skills_load_from_client_resources() {
    use moray_extensions::skills::load_skills_from_dir;

    let skills_dir = bundled_skills_dir();
    let loaded = load_skills_from_dir(&skills_dir).expect("load skills");
    assert!(
        !loaded.is_empty(),
        "expected skills under {}",
        skills_dir.display()
    );
    assert!(
        loaded.iter().any(|s| s.name == "web-fetch"),
        "expected web-fetch skill, got: {:?}",
        loaded.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn build_sonda_loads_bundled_web_fetch_skill() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda().await;
    let skills = sonda.skill_center.catalog();
    assert!(
        skills.iter().any(|s| s.id == "web-fetch"),
        "expected web-fetch in skill center: {skills:?}"
    );
    sonda.shutdown().await;
}

#[tokio::test]
async fn create_and_delete_session_updates_catalog_and_transcript() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda().await;

    let new_id = sonda.create_session("测试会话标题").expect("create session");
    assert!(!new_id.is_empty());
    assert!(session_transcript_path(&new_id).exists());
    assert!(
        sonda
            .session_catalog
            .entries()
            .iter()
            .any(|e| e.session_id == new_id),
        "created session should appear in catalog"
    );

    sonda.delete_session(&new_id).expect("delete session");
    assert!(!session_transcript_path(&new_id).exists());
    assert!(
        !sonda
            .session_catalog
            .entries()
            .iter()
            .any(|e| e.session_id == new_id),
        "deleted session should be removed from catalog"
    );

    sonda.shutdown().await;
}

#[tokio::test]
async fn snapshot_notifies_on_create_and_delete() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda().await;
    let mut events = sonda.snapshot.subscribe();
    let _ = events
        .recv()
        .await
        .expect("initial snapshot");

    let new_id = sonda.create_session("Snapshot test").expect("create");
    let added = events.recv().await.expect("session_added");
    assert!(matches!(added, SondaStateEvent::SessionAdded { .. }));

    sonda.delete_session(&new_id).expect("delete");
    let removed = events.recv().await.expect("session_removed");
    assert!(matches!(removed, SondaStateEvent::SessionRemoved { .. }));

    sonda.shutdown().await;
}

#[tokio::test]
async fn session_agent_binding_roundtrip() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda_with_server_toml(two_agent_server_toml()).await;
    let sid = TEST_SESSION_ID;

    assert_eq!(
        sonda
            .session_catalog
            .get_session_agent_id(sid)
            .expect("get agent"),
        "z9y8x7w6"
    );

    sonda
        .set_session_agent_id(sid, "b2c3d4e5")
        .expect("set agent");
    assert_eq!(
        sonda
            .session_catalog
            .get_session_agent_id(sid)
            .expect("get agent after set"),
        "b2c3d4e5"
    );

    let err = sonda.set_session_agent_id(sid, "xxxxxxxx");
    assert!(err.is_err(), "unknown agent should fail");

    sonda.shutdown().await;
    let _ = std::fs::remove_file(session_transcript_path(sid));
}

#[tokio::test]
async fn transcript_replays_reset_event() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda().await;
    let sid = TEST_SESSION_ID;

    sonda
        .live_sessions
        .reset(sid)
        .await
        .expect("reset session");

    let mut events = sonda
        .session_transcripts
        .subscribe(sid, 0)
        .expect("subscribe");

    let mut saw_reset = false;
    for _ in 0..16 {
        let next = timeout(Duration::from_secs(2), events.recv()).await;
        match next {
            Ok(Some(record)) => {
                let kind = format!("{:?}", record.event);
                if kind.contains("Reset") {
                    saw_reset = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(saw_reset, "transcript replay should include reset event");

    sonda.shutdown().await;
    let _ = std::fs::remove_file(session_transcript_path(sid));
}

#[tokio::test]
async fn live_sessions_cancel_succeeds_for_catalog_session() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda().await;

    sonda
        .live_sessions
        .cancel(TEST_SESSION_ID)
        .expect("cancel idle session");

    sonda.shutdown().await;
    let _ = std::fs::remove_file(session_transcript_path(TEST_SESSION_ID));
}

#[tokio::test]
async fn tool_auth_reply_without_pending_call_errors() {
    let _home = TestHomeGuard::new();
    let sonda = build_test_sonda().await;

    let err = sonda
        .authorizer
        .reply("call-1", serde_json::json!(true))
        .await
        .expect_err("reply without pending call should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("no pending tool authorization"),
        "unexpected error: {msg}"
    );

    sonda.shutdown().await;
    let _ = std::fs::remove_file(session_transcript_path(TEST_SESSION_ID));
}
