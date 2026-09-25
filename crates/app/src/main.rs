mod editor;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod network;
mod runtime_host;
mod theme;
#[cfg(windows)]
mod tray;
mod workspace;

use std::{path::PathBuf, sync::Arc};

use config_store::{ConfigDocument, ConfigStore, DomainConfig};
use gpui_kit::component::Root;
use gpui_kit::{App, AppContext as _, WindowBounds, WindowHandle, WindowOptions, px, size};
use tunnel_core::ManagerHandle;

#[cfg(windows)]
use gpui_kit::{Global, QuitMode, Task};
#[cfg(windows)]
use instance::{InstanceGuard, InstanceStart};
#[cfg(windows)]
use tokio::sync::mpsc;
#[cfg(windows)]
use tray::{TrayAction, TrayController};
#[cfg(windows)]
use tunnel_core::{CoreCommand, ManagerSnapshot, TunnelAction};

type Startup = Result<(PathBuf, DomainConfig), String>;

#[cfg(windows)]
struct DesktopShell {
    _instance: InstanceGuard,
    tray: Option<TrayController>,
    window: Option<WindowHandle<Root>>,
    _actions: Task<()>,
    _state_updates: Option<Task<()>>,
}

#[cfg(windows)]
impl Global for DesktopShell {}

fn main() {
    #[cfg(windows)]
    let (open_sender, mut open_receiver) = mpsc::channel(8);
    #[cfg(windows)]
    let instance = match InstanceGuard::acquire(open_sender) {
        Ok(InstanceStart::Primary(instance)) => instance,
        Ok(InstanceStart::Existing) => return,
        Err(error) => {
            eprintln!("Could not establish single-instance guard: {error}");
            return;
        }
    };
    let background = std::env::args_os().any(|arg| arg == "--background");
    let startup: Arc<Startup> = Arc::new(
        ConfigStore::default_windows()
            .and_then(|store| {
                let directory = store.directory().to_path_buf();
                store.load().map(|config| (directory, config))
            })
            .map_err(|error| error.to_string()),
    );

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (runtime, runtime_error) = match startup.as_ref() {
                Ok((directory, config)) => {
                    match runtime_host::RuntimeHost::new(config.clone(), directory.clone()) {
                        Ok(runtime) => (Some(runtime), None),
                        Err(error) => (None, Some(error)),
                    }
                }
                Err(_) => (None, None),
            };
            let manager = runtime.as_ref().map(runtime_host::RuntimeHost::handle);
            if let Some(runtime) = runtime {
                cx.set_global(runtime);
            }

            #[cfg(windows)]
            {
                let (tray_sender, mut tray_receiver) = mpsc::channel(32);
                let config = manager
                    .as_ref()
                    .map(|handle| handle.subscribe_config().borrow().as_ref().clone())
                    .or_else(|| {
                        startup
                            .as_ref()
                            .as_ref()
                            .ok()
                            .map(|(_, config)| config.clone())
                    })
                    .or_else(|| ConfigDocument::default().into_domain().ok());
                let snapshot = manager
                    .as_ref()
                    .map(|handle| handle.subscribe().borrow().clone())
                    .unwrap_or_else(|| std::sync::Arc::new(ManagerSnapshot::new()));
                let tray = config.as_ref().and_then(|config| {
                    match TrayController::new(tray_sender, config, &snapshot) {
                        Ok(tray) => Some(tray),
                        Err(error) => {
                            eprintln!("Could not create tray icon: {error}");
                            None
                        }
                    }
                });
                cx.set_quit_mode(if tray.is_some() {
                    QuitMode::Explicit
                } else {
                    QuitMode::LastWindowClosed
                });
                let initial_window = if !background || tray.is_none() {
                    match open_main_window(
                        cx,
                        startup.as_ref().clone(),
                        manager.clone(),
                        runtime_error.clone(),
                    ) {
                        Ok(window) => Some(window),
                        Err(error) => {
                            eprintln!("Could not open TunnelWarden window: {error}");
                            None
                        }
                    }
                } else {
                    None
                };
                let action_manager = manager.clone();
                let action_startup = Arc::clone(&startup);
                let action_runtime_error = runtime_error.clone();
                let actions = cx.spawn(async move |cx| {
                    loop {
                        let action = tokio::select! {
                            action = tray_receiver.recv() => action,
                            open = open_receiver.recv() => open.map(|_| TrayAction::Open),
                        };
                        let Some(action) = action else {
                            break;
                        };
                        match action {
                            TrayAction::Open => {
                                let startup = action_startup.clone();
                                let manager = action_manager.clone();
                                let error = action_runtime_error.clone();
                                cx.update(|app| show_main_window(app, startup, manager, error));
                            }
                            TrayAction::StartAll => {
                                if let Some(handle) = &action_manager {
                                    let _ = handle.try_send(CoreCommand::StartAll);
                                }
                            }
                            TrayAction::StopAll => {
                                if let Some(handle) = &action_manager {
                                    let _ = handle.try_send(CoreCommand::StopAll);
                                }
                            }
                            TrayAction::Tunnel(id, action) => {
                                if let Some(handle) = &action_manager {
                                    let command = match action {
                                        TunnelAction::Start => CoreCommand::StartTunnel(id),
                                        TunnelAction::Stop => CoreCommand::StopTunnel(id),
                                        TunnelAction::Retry => CoreCommand::RetryTunnel(id),
                                    };
                                    if let Err(error) = handle.try_send(command) {
                                        eprintln!("Tray tunnel action was not accepted: {error}");
                                    }
                                }
                            }
                            TrayAction::Quit => {
                                cx.update(|app| app.quit());
                                break;
                            }
                        }
                    }
                });
                let state_updates = if let Some(handle) = manager.clone() {
                    let mut configs = handle.subscribe_config();
                    let mut snapshots = handle.subscribe();
                    let mut host_keys = handle.subscribe_host_keys();
                    let state_startup = Arc::clone(&startup);
                    let state_manager = manager.clone();
                    let state_runtime_error = runtime_error.clone();
                    Some(cx.spawn(async move |cx| {
                        let mut opened_prompt_id = None;
                        let initial_prompt_id = host_keys.borrow().first().map(|prompt| prompt.id);
                        if let Some(id) = initial_prompt_id {
                            opened_prompt_id = Some(id);
                            cx.update(|app| {
                                show_main_window(
                                    app,
                                    state_startup.clone(),
                                    state_manager.clone(),
                                    state_runtime_error.clone(),
                                )
                            });
                        }
                        loop {
                            tokio::select! {
                                result = configs.changed() => if result.is_err() { break; },
                                result = snapshots.changed() => if result.is_err() { break; },
                                result = host_keys.changed() => if result.is_err() { break; },
                            }
                            let config = std::sync::Arc::clone(&configs.borrow_and_update());
                            let snapshot = std::sync::Arc::clone(&snapshots.borrow_and_update());
                            let pending_prompt_id = host_keys
                                .borrow_and_update()
                                .first()
                                .map(|prompt| prompt.id);
                            let new_prompt = pending_prompt_id.is_some()
                                && pending_prompt_id != opened_prompt_id;
                            opened_prompt_id = pending_prompt_id;
                            cx.update(|app| {
                                if let Some(tray) = app.global_mut::<DesktopShell>().tray.as_mut() {
                                    let _ = tray.update(&config, &snapshot);
                                }
                                if new_prompt {
                                    show_main_window(
                                        app,
                                        state_startup.clone(),
                                        state_manager.clone(),
                                        state_runtime_error.clone(),
                                    );
                                }
                            });
                        }
                    }))
                } else {
                    None
                };
                cx.set_global(DesktopShell {
                    _instance: instance,
                    tray,
                    window: initial_window,
                    _actions: actions,
                    _state_updates: state_updates,
                });
            }
            #[cfg(not(windows))]
            if let Err(error) =
                open_main_window(cx, startup.as_ref().clone(), manager, runtime_error)
            {
                eprintln!("Could not open TunnelWarden window: {error}");
            }
        });
}

fn open_main_window(
    cx: &mut App,
    startup: Startup,
    manager: Option<ManagerHandle>,
    runtime_error: Option<String>,
) -> Result<WindowHandle<Root>, String> {
    #[cfg(windows)]
    let close_manager = manager.clone();
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
            window_min_size: Some(size(px(920.), px(600.))),
            ..WindowOptions::default()
        },
        |window, cx| {
            #[cfg(windows)]
            window.on_window_should_close(cx, move |_, app| {
                let keep_running = close_manager
                    .as_ref()
                    .is_some_and(|handle| handle.subscribe_config().borrow().app.minimize_to_tray);
                if keep_running {
                    true
                } else {
                    app.quit();
                    false
                }
            });
            let view =
                cx.new(|cx| workspace::Workspace::new(startup, manager, runtime_error, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn show_main_window(
    cx: &mut App,
    startup: Arc<Startup>,
    manager: Option<ManagerHandle>,
    runtime_error: Option<String>,
) {
    let old = cx.global::<DesktopShell>().window;
    if let Some(old) = old
        && old
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let startup = match (startup.as_ref(), &manager) {
        (Ok((directory, _)), Some(handle)) => Ok((
            directory.clone(),
            handle.subscribe_config().borrow().as_ref().clone(),
        )),
        (other, _) => other.clone(),
    };
    match open_main_window(cx, startup, manager, runtime_error) {
        Ok(window) => cx.global_mut::<DesktopShell>().window = Some(window),
        Err(error) => eprintln!("Could not open TunnelWarden window: {error}"),
    }
}
