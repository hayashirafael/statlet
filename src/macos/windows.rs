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
    pub summaries: PreviewSummaries,
    pub layout: IndicatorLayoutDiagnostics,
    pub environment: VisualEnvironment,
}

pub struct PreviewSummaries {
    pub light: String,
    pub dark: String,
}

pub struct WindowManager {
    control_target: Retained<ControlTarget>,
    preferences: Option<PreferencesWindow>,
    history: Option<HistoryWindow>,
    free_space: Option<FreeSpaceWindow>,
}

trait RetainedStateConsumer {
    fn apply_retained_state(&self, state: &AppState);
}

impl RetainedStateConsumer for PreferencesWindow {
    fn apply_retained_state(&self, state: &AppState) {
        self.apply(state, None);
    }
}

impl RetainedStateConsumer for FreeSpaceWindow {
    fn apply_retained_state(&self, state: &AppState) {
        self.apply(state);
    }
}

fn prepare_preferences_for_show<'a, P, F>(
    preferences: &'a mut Option<P>,
    free_space: Option<&F>,
    state: &AppState,
    create: impl FnOnce() -> P,
) -> &'a P
where
    P: RetainedStateConsumer,
    F: RetainedStateConsumer,
{
    get_or_create_window(preferences, create);
    apply_state_to_retained_windows(preferences.as_ref(), free_space, state);
    preferences
        .as_ref()
        .expect("preferences window was created before applying state")
}

fn apply_state_to_retained_windows<P, F>(
    preferences: Option<&P>,
    free_space: Option<&F>,
    state: &AppState,
) where
    P: RetainedStateConsumer,
    F: RetainedStateConsumer,
{
    if let Some(window) = preferences {
        window.apply_retained_state(state);
    }
    if let Some(window) = free_space {
        window.apply_retained_state(state);
    }
}

fn release_preferences<P>(preferences: &mut Option<P>) {
    drop(preferences.take());
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
                let preferences = prepare_preferences_for_show(
                    &mut self.preferences,
                    self.free_space.as_ref(),
                    state,
                    || PreferencesWindow::new(mtm, &target),
                );
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
        apply_state_to_retained_windows(self.preferences.as_ref(), self.free_space.as_ref(), state);
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

    pub fn release_preferences(&mut self) {
        release_preferences(&mut self.preferences);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use statlet::core::{AppState, StatletCore};

    use super::{prepare_preferences_for_show, release_preferences, RetainedStateConsumer};

    #[derive(Default)]
    struct RecordingStateConsumer {
        applications: RefCell<Vec<AppState>>,
    }

    impl RetainedStateConsumer for RecordingStateConsumer {
        fn apply_retained_state(&self, state: &AppState) {
            self.applications.borrow_mut().push(state.clone());
        }
    }

    #[test]
    fn opening_preferences_refreshes_a_retained_free_space_window() {
        let state = StatletCore::new().state().clone();
        let mut preferences = None;
        let free_space = RecordingStateConsumer::default();

        let preferences = prepare_preferences_for_show(
            &mut preferences,
            Some(&free_space),
            &state,
            RecordingStateConsumer::default,
        );

        assert_eq!(
            preferences.applications.borrow().as_slice(),
            std::slice::from_ref(&state)
        );
        assert_eq!(free_space.applications.borrow().len(), 1);
        assert_eq!(
            free_space.applications.borrow().as_slice(),
            std::slice::from_ref(&state)
        );
    }

    #[test]
    fn closing_then_reopening_preferences_rebuilds_from_the_latest_app_state() {
        let mut preferences = Some(RecordingStateConsumer::default());
        release_preferences(&mut preferences);
        assert!(preferences.is_none());

        let mut core = StatletCore::new();
        core.handle(statlet::core::AppEvent::SetMoleIntegrationEnabled(true));
        let latest = core.state().clone();
        let reopened = prepare_preferences_for_show(
            &mut preferences,
            None::<&RecordingStateConsumer>,
            &latest,
            RecordingStateConsumer::default,
        );

        assert_eq!(
            reopened.applications.borrow().as_slice(),
            std::slice::from_ref(&latest)
        );
    }
}
