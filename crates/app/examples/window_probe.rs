#[cfg(windows)]
#[path = "../src/instance.rs"]
mod instance;

#[cfg(windows)]
mod app {
    use super::instance::{InstanceGuard, InstanceStart};
    use gpui_kit::component::Root;
    use gpui_kit::{
        App, AppContext as _, Context, Global, IntoElement, ParentElement as _, QuitMode, Render,
        Styled as _, Task, Window, WindowBounds, WindowHandle, WindowOptions, div, px, size,
    };
    use tokio::sync::mpsc;

    struct ProbeView;
    impl Render for ProbeView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child("Window resource probe")
        }
    }

    struct Owner {
        _instance: InstanceGuard,
        _task: Task<()>,
        window: Option<WindowHandle<Root>>,
    }
    impl Global for Owner {}

    pub fn run() {
        let (sender, mut receiver) = mpsc::channel(8);
        let instance = match InstanceGuard::acquire(sender) {
            Ok(InstanceStart::Primary(guard)) => guard,
            Ok(InstanceStart::Existing) => return,
            Err(error) => {
                eprintln!("Probe single instance: {error}");
                return;
            }
        };
        gpui_kit::application()
            .with_assets(gpui_kit::assets::Assets)
            .run(move |cx| {
                gpui_kit::init(cx);
                cx.set_quit_mode(QuitMode::Explicit);
                let task = cx.spawn(async move |cx| {
                    while receiver.recv().await.is_some() {
                        cx.update(|app| {
                            let old = app.global::<Owner>().window;
                            if let Some(old) = old
                                && old
                                    .update(app, |_, window, _| window.activate_window())
                                    .is_ok()
                            {
                                return;
                            }
                            let result = open(app);
                            if let Ok(window) = result {
                                app.global_mut::<Owner>().window = Some(window);
                            }
                        });
                    }
                });
                cx.set_global(Owner {
                    _instance: instance,
                    _task: task,
                    window: None,
                });
            });
    }

    fn open(cx: &mut App) -> Result<WindowHandle<Root>, String> {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
                ..WindowOptions::default()
            },
            |window, cx| {
                let view = cx.new(|_| ProbeView);
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {}
