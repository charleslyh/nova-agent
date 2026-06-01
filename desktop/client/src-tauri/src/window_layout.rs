//! Persist and restore window layout per monitor work area.
//!
//! All attached windows share one JSON store file; the host app chooses the path
//! via [`configure_store`] and which windows to track via [`attach_window`].
//!
//! # macOS (handled in [`geometry::apply`])
//! - `set_position(Physical)` uses the window's *current* screen scale → use
//!   [`geometry::set_outer_position`] with the **target** monitor scale.
//! - Hidden windows may not hop displays until `show()`.
//! - `set_inner_size` grows from the frame bottom-left → re-apply position after resize.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize,
    RunEvent, Runtime, WebviewWindow, WindowEvent,
};

const DEFAULT_SAVE_DEBOUNCE_MS: u64 = 400;
const DEFAULT_STARTUP_PERSIST_GRACE_MS: u64 = 2000;
const DEFAULT_RESTORE_WAIT_MS: &[u64] = &[0, 50, 150];
const DEFAULT_FRAME_SETTLE_MS: u64 = 32;

// ---------------------------------------------------------------------------
// Attach options (host app)
// ---------------------------------------------------------------------------

/// Per-window attach options.
#[derive(Debug, Clone, Default)]
pub struct AttachOptions {
    /// Minimum inner size in **logical** pixels when resolving restore geometry.
    /// If unset, only clamps to the monitor work area (no app-specific floor).
    pub min_inner_logical: Option<(f64, f64)>,
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// On-disk document: one file, many windows keyed by label.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LayoutStore {
    #[serde(default)]
    windows: HashMap<String, SavedLayout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedLayout {
    monitor_name: Option<String>,
    #[serde(default)]
    monitor_work_area_x: Option<i32>,
    #[serde(default)]
    monitor_work_area_y: Option<i32>,
    #[serde(default)]
    monitor_work_area_width: Option<u32>,
    #[serde(default)]
    monitor_work_area_height: Option<u32>,
    #[serde(default)]
    monitor_scale_factor: Option<f64>,
    #[serde(default)]
    frame_width_overhang: Option<u32>,
    #[serde(default)]
    frame_height_overhang: Option<u32>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy)]
struct MonitorRef {
    work_area_x: i32,
    work_area_y: i32,
    work_area_width: i32,
    work_area_height: i32,
    scale_factor: f64,
}

impl MonitorRef {
    fn from(monitor: &Monitor) -> Self {
        let area = monitor.work_area();
        Self {
            work_area_x: area.position.x,
            work_area_y: area.position.y,
            work_area_width: area.size.width as i32,
            work_area_height: area.size.height as i32,
            scale_factor: monitor.scale_factor(),
        }
    }

    fn matches_saved(&self, saved: &SavedLayout) -> bool {
        saved.monitor_work_area_x == Some(self.work_area_x)
            && saved.monitor_work_area_y == Some(self.work_area_y)
            && saved.monitor_work_area_width == Some(self.work_area_width as u32)
            && saved.monitor_work_area_height == Some(self.work_area_height as u32)
            && saved
                .monitor_scale_factor
                .is_some_and(|s| (s - self.scale_factor).abs() < 0.01)
    }
}

struct ResolvedGeometry {
    outer_position: PhysicalPosition<i32>,
    inner_size: PhysicalSize<u32>,
    scale_factor: f64,
}

struct WindowSession {
    min_inner_logical: Option<(f64, f64)>,
    restoring: Arc<AtomicBool>,
    allow_persist: Arc<AtomicBool>,
    attached: AtomicBool,
}

impl WindowSession {
    fn new(options: &AttachOptions) -> Self {
        Self {
            min_inner_logical: options.min_inner_logical,
            restoring: Arc::new(AtomicBool::new(false)),
            allow_persist: Arc::new(AtomicBool::new(false)),
            attached: AtomicBool::new(false),
        }
    }
}

struct PluginState {
    layout_file_path: Mutex<Option<PathBuf>>,
    store: Mutex<LayoutStore>,
    persist_generation: Arc<AtomicU64>,
    exit_saved: AtomicBool,
    windows: Mutex<HashMap<String, Arc<WindowSession>>>,
}

// ---------------------------------------------------------------------------
// Disk I/O
// ---------------------------------------------------------------------------

fn ensure_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

/// Load store from disk (`{ "windows": { "<label>": ... } }` only).
fn load_store(path: &Path) -> LayoutStore {
    let Ok(data) = std::fs::read_to_string(path) else {
        return LayoutStore::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_store(path: &Path, store: &LayoutStore) {
    ensure_parent_dir(path);
    if let Ok(bytes) = serde_json::to_vec_pretty(store) {
        let _ = std::fs::write(path, bytes);
    }
}

fn store_path(state: &PluginState) -> Option<PathBuf> {
    state.layout_file_path.lock().unwrap().clone()
}

/// Set the layout JSON path and load existing state. Call once from app `setup`
/// before [`attach_window`].
pub fn configure_store<R: Runtime>(app: &AppHandle<R>, path: PathBuf) {
    ensure_parent_dir(&path);
    let state = plugin_state(app);
    *state.layout_file_path.lock().unwrap() = Some(path.clone());
    *state.store.lock().unwrap() = load_store(&path);
}

// ---------------------------------------------------------------------------
// Monitor matching
// ---------------------------------------------------------------------------

fn intersection_area(monitor: &Monitor, x: i32, y: i32, w: u32, h: u32) -> i64 {
    let area = monitor.work_area();
    let left = x.max(area.position.x);
    let top = y.max(area.position.y);
    let right = (x + w as i32).min(area.position.x + area.size.width as i32);
    let bottom = (y + h as i32).min(area.position.y + area.size.height as i32);
    ((right - left).max(0) as i64) * ((bottom - top).max(0) as i64)
}

fn monitor_for_window<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<Option<Monitor>> {
    let position = window.outer_position()?;
    let size = window.inner_size()?;
    if size.width == 0 || size.height == 0 {
        return Ok(None);
    }
    Ok(window
        .available_monitors()?
        .into_iter()
        .max_by_key(|m| intersection_area(m, position.x, position.y, size.width, size.height)))
}

fn find_monitor_for_layout<'a>(
    monitors: &'a [Monitor],
    saved: &SavedLayout,
) -> Option<&'a Monitor> {
    if let Some(name) = &saved.monitor_name {
        if let Some(m) = monitors.iter().find(|m| m.name().is_some_and(|n| n == name)) {
            return Some(m);
        }
    }
    monitors
        .iter()
        .find(|m| MonitorRef::from(m).matches_saved(saved))
}

// ---------------------------------------------------------------------------
// Capture & resolve geometry
// ---------------------------------------------------------------------------

mod geometry {
    use super::*;

    fn min_inner_physical(monitor: &Monitor, min_logical: Option<(f64, f64)>) -> (u32, u32) {
        let Some((w, h)) = min_logical else {
            return (1, 1);
        };
        let s = monitor.scale_factor();
        (
            (w * s).round().max(1.0) as u32,
            (h * s).round().max(1.0) as u32,
        )
    }

    pub fn capture<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<Option<SavedLayout>> {
        let monitor = match monitor_for_window(window)? {
            Some(m) => m,
            None => return Ok(None),
        };
        let mref = MonitorRef::from(&monitor);
        let wa_w = mref.work_area_width;
        let wa_h = mref.work_area_height;
        if wa_w <= 0 || wa_h <= 0 {
            return Ok(None);
        }

        let position = window.outer_position()?;
        let inner = window.inner_size()?;
        let outer = window.outer_size()?;
        if inner.width == 0 || inner.height == 0 {
            return Ok(None);
        }
        if inner.width > wa_w as u32 * 2 || inner.height > wa_h as u32 * 2 {
            return Ok(None);
        }

        Ok(Some(SavedLayout {
            monitor_name: monitor.name().cloned(),
            monitor_work_area_x: Some(mref.work_area_x),
            monitor_work_area_y: Some(mref.work_area_y),
            monitor_work_area_width: Some(mref.work_area_width as u32),
            monitor_work_area_height: Some(mref.work_area_height as u32),
            monitor_scale_factor: Some(mref.scale_factor),
            frame_width_overhang: Some(outer.width.saturating_sub(inner.width)),
            frame_height_overhang: Some(outer.height.saturating_sub(inner.height)),
            x: ((position.x - mref.work_area_x) as f32 / wa_w as f32).clamp(0.0, 1.0),
            y: ((position.y - mref.work_area_y) as f32 / wa_h as f32).clamp(0.0, 1.0),
            width: (inner.width as f32 / wa_w as f32).clamp(0.1, 1.0),
            height: (inner.height as f32 / wa_h as f32).clamp(0.1, 1.0),
        }))
    }

    pub fn resolve(
        saved: &SavedLayout,
        monitor: &Monitor,
        min_inner_logical: Option<(f64, f64)>,
    ) -> ResolvedGeometry {
        let mref = MonitorRef::from(monitor);
        let (min_w, min_h) = min_inner_physical(monitor, min_inner_logical);
        let fw = saved.frame_width_overhang.unwrap_or(0);
        let fh = saved.frame_height_overhang.unwrap_or(0);

        let mut width = (saved.width * mref.work_area_width as f32)
            .round()
            .max(min_w as f32) as u32;
        let mut height = (saved.height * mref.work_area_height as f32)
            .round()
            .max(min_h as f32) as u32;
        width = width.min(mref.work_area_width as u32);
        height = height.min(mref.work_area_height as u32);

        let mut x = mref.work_area_x + (saved.x * mref.work_area_width as f32).round() as i32;
        let mut y = mref.work_area_y + (saved.y * mref.work_area_height as f32).round() as i32;

        if x + width as i32 + fw as i32 > mref.work_area_x + mref.work_area_width {
            x = mref.work_area_x + mref.work_area_width - width as i32 - fw as i32;
        }
        if y + height as i32 + fh as i32 > mref.work_area_y + mref.work_area_height {
            y = mref.work_area_y + mref.work_area_height - height as i32 - fh as i32;
        }

        ResolvedGeometry {
            outer_position: PhysicalPosition {
                x: x.max(mref.work_area_x),
                y: y.max(mref.work_area_y),
            },
            inner_size: PhysicalSize { width, height },
            scale_factor: mref.scale_factor,
        }
    }

    pub fn set_outer_position<R: Runtime>(
        window: &WebviewWindow<R>,
        position: PhysicalPosition<i32>,
        scale_factor: f64,
    ) -> tauri::Result<()> {
        window.set_position(LogicalPosition::new(
            position.x as f64 / scale_factor,
            position.y as f64 / scale_factor,
        ))
    }

    async fn settle_frame(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    pub async fn apply<R: Runtime>(
        window: &WebviewWindow<R>,
        geom: &ResolvedGeometry,
        frame_settle_ms: u64,
    ) -> bool {
        let logical_size = LogicalSize::new(
            geom.inner_size.width as f64 / geom.scale_factor,
            geom.inner_size.height as f64 / geom.scale_factor,
        );
        let scale = geom.scale_factor;
        let pos = geom.outer_position;

        if set_outer_position(window, pos, scale).is_err() {
            return false;
        }
        settle_frame(frame_settle_ms).await;
        let _ = window.show();
        if set_outer_position(window, pos, scale).is_err() {
            return false;
        }
        settle_frame(frame_settle_ms).await;
        if window.set_size(logical_size).is_err() {
            return false;
        }
        settle_frame(frame_settle_ms).await;
        if set_outer_position(window, pos, scale).is_err() {
            return false;
        }
        settle_frame(frame_settle_ms).await;
        true
    }
}

// ---------------------------------------------------------------------------
// Store operations
// ---------------------------------------------------------------------------

fn plugin_state<R: Runtime>(app: &AppHandle<R>) -> tauri::State<'_, PluginState> {
    app.state::<PluginState>()
}

fn window_session<R: Runtime>(window: &WebviewWindow<R>) -> Option<Arc<WindowSession>> {
    let label = window.label().to_string();
    plugin_state(window.app_handle())
        .windows
        .lock()
        .unwrap()
        .get(&label)
        .cloned()
}

fn saved_layout_for_label(state: &PluginState, label: &str) -> Option<SavedLayout> {
    state.store.lock().unwrap().windows.get(label).cloned()
}

fn register_session<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    options: &AttachOptions,
) -> Arc<WindowSession> {
    let state = plugin_state(app);
    let mut windows = state.windows.lock().unwrap();
    if let Some(existing) = windows.get(label) {
        return existing.clone();
    }
    let session = Arc::new(WindowSession::new(options));
    windows.insert(label.to_string(), session.clone());
    session
}

fn update_window_in_store<R: Runtime>(window: &WebviewWindow<R>, state: &PluginState) -> bool {
    let Ok(Some(layout)) = geometry::capture(window) else {
        return false;
    };
    state
        .store
        .lock()
        .unwrap()
        .windows
        .insert(window.label().to_string(), layout);
    true
}

fn write_store_to_disk(state: &PluginState) {
    let Some(path) = store_path(state) else {
        return;
    };
    let store = state.store.lock().unwrap().clone();
    save_store(&path, &store);
}

fn flush_window_to_disk<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle();
    let state = plugin_state(&app);
    let _ = update_window_in_store(window, &state);
    write_store_to_disk(&state);
}

fn schedule_debounced_save<R: Runtime>(window: &WebviewWindow<R>, debounce_ms: u64) {
    let app = window.app_handle();
    let state = plugin_state(&app);
    let Some(session) = window_session(window) else {
        return;
    };
    if !session.allow_persist.load(Ordering::SeqCst) {
        return;
    }
    if store_path(&state).is_none() {
        return;
    }
    if !update_window_in_store(window, &state) {
        return;
    }

    let generation = state.persist_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    let persist_generation = state.persist_generation.clone();
    let allow_persist = session.allow_persist.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(debounce_ms)).await;
        if persist_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if !allow_persist.load(Ordering::SeqCst) {
            return;
        }
        let state = plugin_state(&app);
        write_store_to_disk(&state);
    });
}

async fn restore_window<R: Runtime>(
    window: WebviewWindow<R>,
    session: Arc<WindowSession>,
    saved: SavedLayout,
) {
    for &delay_ms in DEFAULT_RESTORE_WAIT_MS {
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let Ok(monitors) = window.available_monitors() else {
            continue;
        };
        let Some(monitor) = find_monitor_for_layout(&monitors, &saved) else {
            continue;
        };

        session.restoring.store(true, Ordering::SeqCst);
        let geom = geometry::resolve(&saved, monitor, session.min_inner_logical);
        let ok = geometry::apply(&window, &geom, DEFAULT_FRAME_SETTLE_MS).await;
        session.restoring.store(false, Ordering::SeqCst);

        if ok {
            let _ = window.set_focus();
            session.allow_persist.store(true, Ordering::SeqCst);
            return;
        }
    }

    let _ = window.show();
    let _ = window.set_focus();
    session.allow_persist.store(true, Ordering::SeqCst);
}

fn flush_all_on_exit<R: Runtime>(app: &AppHandle<R>) {
    let state = plugin_state(app);
    if state.exit_saved.swap(true, Ordering::SeqCst) {
        return;
    }
    if store_path(&state).is_none() {
        return;
    }

    state.persist_generation.fetch_add(1, Ordering::SeqCst);

    let labels: Vec<String> = state.windows.lock().unwrap().keys().cloned().collect();
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            let session = window_session(&window);
            if let Some(session) = session {
                session.allow_persist.store(true, Ordering::SeqCst);
            }
            let _ = update_window_in_store(&window, &state);
        }
    }
    write_store_to_disk(&state);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

fn handle_window_event<R: Runtime>(window: &WebviewWindow<R>, event: &WindowEvent) {
    let Some(session) = window_session(window) else {
        return;
    };
    match event {
        WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
            if session.restoring.load(Ordering::SeqCst) {
                return;
            }
            schedule_debounced_save(window, DEFAULT_SAVE_DEBOUNCE_MS);
        }
        WindowEvent::CloseRequested { .. } => flush_on_exit(window),
        _ => {}
    }
}

/// Flush this window's layout into the shared store and write the file once.
pub fn flush_on_exit<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle();
    let state = plugin_state(&app);
    if store_path(&state).is_none() {
        return;
    }
    let Some(session) = window_session(window) else {
        return;
    };
    session.allow_persist.store(true, Ordering::SeqCst);
    session.restoring.store(false, Ordering::SeqCst);
    state.persist_generation.fetch_add(1, Ordering::SeqCst);
    flush_window_to_disk(window);
}

/// Attach save/restore to an existing window. Requires [`configure_store`] first.
pub fn attach_window<R: Runtime>(app: &AppHandle<R>, label: &str, options: AttachOptions) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    let state = plugin_state(app);
    let session = register_session(app, label, &options);
    if session.attached.swap(true, Ordering::SeqCst) {
        return;
    }

    session.allow_persist.store(false, Ordering::SeqCst);

    let saved = saved_layout_for_label(&state, label);
    let window_for_events = window.clone();
    window.on_window_event(move |event| {
        handle_window_event(&window_for_events, &event);
    });

    let allow_persist = session.allow_persist.clone();

    if let Some(layout) = saved {
        tauri::async_runtime::spawn(async move {
            restore_window(window, session, layout).await;
        });
    } else {
        let _ = window.show();
        let _ = window.set_focus();
        allow_persist.store(true, Ordering::SeqCst);
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(DEFAULT_STARTUP_PERSIST_GRACE_MS)).await;
        allow_persist.store(true, Ordering::SeqCst);
    });
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::new("window-layout")
        .setup(|app, _| {
            app.manage(PluginState {
                layout_file_path: Mutex::new(None),
                store: Mutex::new(LayoutStore::default()),
                persist_generation: Arc::new(AtomicU64::new(0)),
                exit_saved: AtomicBool::new(false),
                windows: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .on_event(|app, event| {
            if matches!(event, RunEvent::Exit) {
                flush_all_on_exit(app);
            }
        })
        .build()
}
