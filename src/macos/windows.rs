use objc2::rc::Retained;
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;
use statlet::core::{AppState, WindowKind};
use statlet::history::History;
use statlet::indicator::LayoutDiagnostics;
use statlet::indicator_preferences::FontFamilyPreference;
use tao::event_loop::EventLoopProxy;

use super::environment::VisualEnvironment;
use super::renderer::PreviewImages;
use super::RuntimeEvent;

mod common;
mod free_space;
mod history;
mod preferences;

use common::ControlTarget;
use free_space::{create_free_space_window, FreeSpaceWindow};
use history::{create_history_window, HistoryWindow};
use preferences::{get_or_create_window, PreferencesWindow};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewContrastWarnings {
    pub light: bool,
    pub dark: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorFontFallback {
    pub requested_family: FontFamilyPreference,
    pub resolved_family: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndicatorLayoutDiagnostics {
    pub status: Option<LayoutDiagnostics>,
    pub light: LayoutDiagnostics,
    pub dark: LayoutDiagnostics,
}

pub struct IndicatorSurfaceUpdate {
    pub previews: PreviewImages,
    pub font_fallback: Option<IndicatorFontFallback>,
    pub contrast_warnings: PreviewContrastWarnings,
    pub layout: IndicatorLayoutDiagnostics,
    pub environment: VisualEnvironment,
}

pub struct WindowManager {
    control_target: Retained<ControlTarget>,
    preferences: Option<PreferencesWindow>,
    history: Option<HistoryWindow>,
    free_space: Option<FreeSpaceWindow>,
}

impl WindowManager {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let control_target = ControlTarget::new(mtm, proxy);
        Self {
            control_target,
            preferences: None,
            history: None,
            free_space: None,
        }
    }

    pub fn show(&mut self, kind: WindowKind, state: &AppState, history: &History) {
        let mtm = MainThreadMarker::new().expect("native window actions run on the main thread");
        let window = match kind {
            WindowKind::Preferences => {
                let target = self.control_target.clone();
                let preferences = get_or_create_window(&mut self.preferences, || {
                    PreferencesWindow::new(mtm, &target)
                });
                preferences.apply(state, None);
                &preferences.window
            }
            WindowKind::History => {
                if self.history.is_none() {
                    self.history = Some(create_history_window(mtm, &self.control_target));
                }
                self.update_history(history);
                &self
                    .history
                    .as_ref()
                    .expect("history window was created")
                    .window
            }
            WindowKind::FreeSpace => {
                if self.free_space.is_none() {
                    self.free_space = Some(create_free_space_window(mtm, &self.control_target));
                }
                self.update_state(state);
                &self
                    .free_space
                    .as_ref()
                    .expect("free-space window was created")
                    .window
            }
        };

        let app = NSApplication::sharedApplication(mtm);
        // A window is shown only after an explicit launch, menu choice, or
        // notification click, so request cooperative activation before
        // promoting the retained window.
        app.activate();
        window.makeKeyAndOrderFront(None);
    }

    pub fn update_state(&self, state: &AppState) {
        if let Some(window) = &self.preferences {
            window.apply(state, None);
        }
        if let Some(window) = &self.free_space {
            window.apply(state);
        }
    }

    pub fn update_history(&self, history: &History) {
        if let Some(window) = &self.history {
            window.apply(history);
        }
    }

    pub fn has_preferences_surface(&self) -> bool {
        self.preferences
            .as_ref()
            .is_some_and(PreferencesWindow::is_created_and_visible)
    }

    pub fn update_indicator_surfaces(&self, surfaces: IndicatorSurfaceUpdate) {
        if let Some(window) = &self.preferences {
            window.apply_surfaces(surfaces);
        }
    }
}
