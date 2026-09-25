use config_store::ThemePreference;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window, WindowAppearance};

/// Applies the saved product preference to the native window and GPUI Kit tokens.
pub fn apply_preference(preference: ThemePreference, window: Option<&mut Window>, cx: &mut App) {
    match preference {
        ThemePreference::System => {
            cx.set_window_appearance(None);
            Theme::sync_system_appearance(window, cx);
        }
        ThemePreference::Light => {
            cx.set_window_appearance(Some(WindowAppearance::Light));
            Theme::change(ThemeMode::Light, window, cx);
        }
        ThemePreference::Dark => {
            cx.set_window_appearance(Some(WindowAppearance::Dark));
            Theme::change(ThemeMode::Dark, window, cx);
        }
    }
}
