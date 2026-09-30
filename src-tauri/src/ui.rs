//! Disposable presentation. The service and tray outlive every WebView.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::Manager;
use tunnelwarden_core::service::AppService;

#[derive(Default)]
pub struct UiState {
    pub ready: AtomicBool,
    operations: tokio::sync::Mutex<()>,
}

pub fn show(app: &tauri::AppHandle, only_for_prompt: bool) {
    let app = app.clone();
    // Tauri explicitly requires async/separate-thread creation on Windows.
    // Serialize opens and destruction without blocking the native event loop.
    tauri::async_runtime::spawn(async move {
        let state = app.state::<UiState>();
        let _operation = state.operations.lock().await;
        if app
            .state::<crate::ExitState>()
            .requested
            .load(Ordering::SeqCst)
        {
            return;
        }
        if only_for_prompt {
            let service = app.state::<Arc<AppService>>();
            if service.prompts.snapshot().is_empty() && service.prompts.key_snapshot().is_empty() {
                return;
            }
        }
        let result = if let Some(window) = app.get_webview_window("main") {
            window
                .show()
                .and_then(|_| window.unminimize())
                .and_then(|_| window.set_focus())
        } else {
            state.ready.store(false, Ordering::SeqCst);
            let config = &app.config().app.windows[0];
            tauri::WebviewWindowBuilder::from_config(&app, config)
                .and_then(|builder| builder.build())
                .map(|_| ())
        };
        if let Err(error) = result {
            app.state::<Arc<AppService>>().logs.push(
                "error",
                None,
                format!("Cannot open interface: {error}"),
            );
        }
    });
}

pub async fn close(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<UiState>();
    let _operation = state.operations.lock().await;
    let service = app.state::<Arc<AppService>>();
    service.cancel_ui_questions();
    if !service.get_settings().close_to_tray {
        app.exit(0);
        return Ok(());
    }
    state.ready.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        window.destroy().map_err(|e| e.to_string())?;
        drop(window);
        // Wait for native removal before a simultaneous tray click can reopen.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while app.get_webview_window("main").is_some() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| "Window destruction timed out".to_string())?;
    }
    Ok(())
}
