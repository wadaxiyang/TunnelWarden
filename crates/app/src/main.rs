mod workspace;

use config_store::ConfigStore;
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, WindowBounds, WindowOptions, px, size};

fn main() {
    let startup = ConfigStore::default_windows().and_then(|store| {
        let directory = store.directory().to_path_buf();
        store.load().map(|config| (directory, config))
    });
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
                    window_min_size: Some(size(px(920.), px(600.))),
                    ..WindowOptions::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| workspace::Workspace::new(startup, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            );
            if let Err(error) = opened {
                eprintln!("Could not open TunnelWarden window: {error}");
            }
        });
}
