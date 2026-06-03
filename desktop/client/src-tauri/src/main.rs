#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod window_layout;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{App, AppHandle, Manager, RunEvent, State};
use tokio::sync::Mutex;
use tracing::warn;

use moray_desktop_client::bundle;
use moray_desktop_client::log::init_tracing;
use moray_desktop_client::sonda;
use moray_desktop_server::SondaGateway;
use moray_sonda::Sonda;

struct SharedServer {
    gateway: Arc<Mutex<Option<SondaGateway>>>,
    shutting_down: Arc<AtomicBool>,
}

#[tauri::command]
async fn get_server_url(state: State<'_, SharedServer>) -> Result<String, String> {
    let guard = state.gateway.lock().await;
    let handle = guard
        .as_ref()
        .ok_or_else(|| "server not started".to_string())?;
    Ok(format!("http://{}", handle.local_addr))
}

fn init_process() {
    init_tracing();
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn create_shared_server() -> SharedServer {
    SharedServer {
        gateway: Arc::new(Mutex::new(None)),
        shutting_down: Arc::new(AtomicBool::new(false)),
    }
}

fn setup_window_layout(app: &App) {
    let layout_path = app
        .path()
        .app_config_dir()
        .expect("app config dir")
        .join(".window-layout.json");
    window_layout::configure_store(app.handle(), layout_path);
    window_layout::attach_window(
        app.handle(),
        "main",
        window_layout::AttachOptions {
            min_inner_logical: Some((900.0, 550.0)),
        },
    );
}

fn start_gateway(gateway: Arc<Mutex<Option<SondaGateway>>>, sonda: Arc<Sonda>) {
    tauri::async_runtime::block_on(async move {
        let mut guard = gateway.lock().await;
        *guard = Some(
            SondaGateway::spawn(sonda)
                .await
                .expect("server start failed"),
        );
    });
}

fn startup_app(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    setup_window_layout(app);

    let paths = bundle::resolve_runtime_paths(app.handle())
        .expect("failed to resolve runtime paths");

    let sonda = Arc::new(sonda::build_sonda(&paths).expect("sonda build failed"));
    app.manage(sonda.clone());
    tauri::async_runtime::block_on(sonda.startup());

    let gateway = app.state::<SharedServer>().gateway.clone();
    start_gateway(gateway, sonda);

    Ok(())
}

async fn stop_gateway(gateway: Arc<Mutex<Option<SondaGateway>>>) {
    let mut guard = gateway.lock().await;
    if let Some(handle) = guard.take() {
        handle.shutdown().await;
    } else {
        warn!("desktop shutdown: gateway not running");
    }
}

/// 返回 `true` 表示已接管退出流程，调用方应 `prevent_exit`。
fn shutdown_app(app_handle: &AppHandle) -> bool {
    let Some(state) = app_handle.try_state::<SharedServer>() else {
        warn!("desktop shutdown: SharedServer state missing");
        return false;
    };
    if state.shutting_down.load(Ordering::SeqCst) {
        return false;
    }
    state.shutting_down.store(true, Ordering::SeqCst);

    let sonda = app_handle
        .try_state::<Arc<Sonda>>()
        .map(|sonda| Arc::clone(&*sonda));
    let app = app_handle.clone();

    let gateway = state.gateway.clone();
    tauri::async_runtime::spawn(async move {
        stop_gateway(gateway).await;
        if let Some(sonda) = sonda {
            sonda.shutdown().await;
        }
        app.exit(0);
    });

    true
}

fn handle_run_event(app_handle: &AppHandle, event: RunEvent) {
    if let RunEvent::ExitRequested { api, .. } = event {
        if shutdown_app(app_handle) {
            api.prevent_exit();
        }
    }
}

fn main() {
    init_process();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(window_layout::init())
        .manage(create_shared_server())
        .invoke_handler(tauri::generate_handler![get_server_url])
        .setup(startup_app)
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| handle_run_event(app_handle, event));
}
