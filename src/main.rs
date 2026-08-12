//! Statlet macOS runtime.
//!
//! Event-loop structure derived and modified from featherbar commit 90ab504,
//! licensed under Apache-2.0.

mod macos;

use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAccessibilityHighContrastAqua,
    NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua,
};
use statlet::core::{AppEffect, AppEvent, Preferences, PreferencesSaveResult, StatletCore};
use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};
use statlet::disk::DiskSamplingSchedule;
use statlet::history::{History, HistoryStore};
use statlet::indicator::{
    compose_indicator, has_low_text_contrast, preview_accessibility_summary, PreviewBackground,
};
use statlet::indicator_preferences::{IndicatorAppearance, MetricsRefreshInterval};
use statlet::metrics_schedule::MetricsSamplingSchedule;
use statlet::mole::{MoleDetection, MoleDetector, MoleInstallation, MoleStatus};
use statlet::preferences::PreferencesStore;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use macos::environment::{PreviewAppearanceName, VisualEnvironment, VisualEnvironmentObserver};
use macos::notifications::NotificationManager;
use macos::renderer::{resolved_scene_srgb_colors, PreviewImages, RenderSlot, Renderer};
use macos::sampler::MacSampler;
use macos::windows::{
    IndicatorFontFallback, IndicatorLayoutDiagnostics, IndicatorSurfaceUpdate,
    PreviewContrastWarnings, PreviewSummaries, WindowManager,
};
use macos::RuntimeEvent;

fn main() {
    let mut event_loop = EventLoopBuilder::<RuntimeEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();

    let review_space_item = MenuItem::new("Revisar espaço…", false, None);
    let review_space_id: MenuId = review_space_item.id().clone();
    let preferences_item = MenuItem::new("Preferências…", true, None);
    let preferences_id: MenuId = preferences_item.id().clone();
    let history_item = MenuItem::new("Histórico…", true, None);
    let history_id: MenuId = history_item.id().clone();
    let quit = MenuItem::new("Sair", true, None);
    let quit_id: MenuId = quit.id().clone();
    let menu = Menu::new();
    menu.append(&review_space_item).expect("build menu");
    menu.append(&preferences_item).expect("build menu");
    menu.append(&history_item).expect("build menu");
    menu.append(&quit).expect("build menu");

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let runtime_event = if event.id == review_space_id {
            Some(RuntimeEvent::App(AppEvent::ReviewSpace))
        } else if event.id == preferences_id {
            Some(RuntimeEvent::App(AppEvent::OpenPreferences))
        } else if event.id == history_id {
            Some(RuntimeEvent::App(AppEvent::OpenHistory))
        } else if event.id == quit_id {
            Some(RuntimeEvent::App(AppEvent::Quit))
        } else {
            None
        };
        if let Some(event) = runtime_event {
            let _ = menu_proxy.send_event(event);
        }
    }));

    let preferences_store = PreferencesStore::for_current_user()
        .expect("resolve the current user's preferences directory");
    let history_store =
        HistoryStore::for_current_user().expect("resolve the current user's history directory");
    let history = history_store.load();
    let initial_preferences = preferences_store.load();
    let initial_metrics_interval = initial_preferences.indicator.refresh_interval;
    let (mut core, startup_effects) = StatletCore::with_preferences(initial_preferences);
    let mut runtime = RuntimeAdapters::new(
        preferences_store,
        history_store,
        history,
        review_space_item,
        proxy.clone(),
        initial_metrics_interval,
    );
    let mut renderer = Renderer::new();
    // tray-icon removes the status item when its owner is dropped.
    let mut _retained_tray: Option<TrayIcon> = None;
    let mut button = None;

    event_loop.run(move |event, _target, control_flow| match event {
        Event::NewEvents(StartCause::Init) => {
            _retained_tray = match TrayIconBuilder::new()
                .with_menu(Box::new(menu.clone()))
                .build()
            {
                Ok(tray) => Some(tray),
                Err(error) => {
                    eprintln!("Statlet could not create its status item: {error}");
                    None
                }
            };
            let marker = MainThreadMarker::new().expect("main-thread event loop");
            button = macos::renderer::status_button(marker);
            runtime.initialize_native(marker, proxy.clone(), button.as_deref());
            let _ = proxy.send_event(RuntimeEvent::App(AppEvent::ApplicationLaunched));
            let _ = runtime.apply_effects(
                &startup_effects,
                &mut core,
                &mut renderer,
                button.as_deref(),
            );
            let sampling_effects = run_redraw_cycle(
                RedrawReason::Metrics,
                &mut runtime,
                &mut core,
                &mut renderer,
                button.as_deref(),
            );
            let _ = runtime.apply_effects(
                &sampling_effects,
                &mut core,
                &mut renderer,
                button.as_deref(),
            );
            set_next_wakeup(control_flow, &runtime.samplers);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            if button.is_none() {
                let marker = MainThreadMarker::new().expect("main-thread event loop");
                button = macos::renderer::status_button(marker);
                runtime.rebind_status_button(button.as_deref());
            }
            let sampling_effects = run_redraw_cycle(
                RedrawReason::Metrics,
                &mut runtime,
                &mut core,
                &mut renderer,
                button.as_deref(),
            );
            let _ = runtime.apply_effects(
                &sampling_effects,
                &mut core,
                &mut renderer,
                button.as_deref(),
            );
            set_next_wakeup(control_flow, &runtime.samplers);
        }
        Event::UserEvent(runtime_event) => {
            let effects = match runtime_event {
                RuntimeEvent::App(app_event) => core.handle(app_event),
                RuntimeEvent::VisualEnvironmentChanged => {
                    let marker = MainThreadMarker::new().expect("visual events run on main thread");
                    let environment = VisualEnvironment::current(button.as_deref(), marker);
                    if runtime.visual_environment.record(environment) {
                        run_redraw_cycle(
                            RedrawReason::Appearance,
                            &mut runtime,
                            &mut core,
                            &mut renderer,
                            button.as_deref(),
                        )
                    } else {
                        Vec::new()
                    }
                }
                RuntimeEvent::FontSetChanged => run_redraw_cycle(
                    RedrawReason::Fonts,
                    &mut runtime,
                    &mut core,
                    &mut renderer,
                    button.as_deref(),
                ),
                RuntimeEvent::ScreenParametersChanged => run_redraw_cycle(
                    RedrawReason::Screens,
                    &mut runtime,
                    &mut core,
                    &mut renderer,
                    button.as_deref(),
                ),
                RuntimeEvent::MoleDetected {
                    generation,
                    detection,
                } => runtime
                    .mole
                    .apply_detection(generation, detection)
                    .map(|status| core.handle(AppEvent::MoleStatusObserved(status)))
                    .unwrap_or_default(),
            };
            if runtime.apply_effects(&effects, &mut core, &mut renderer, button.as_deref()) {
                *control_flow = ControlFlow::Exit;
            } else {
                set_next_wakeup(control_flow, &runtime.samplers);
            }
        }
        Event::Reopen {
            has_visible_windows,
            ..
        } => {
            let effects = core.handle(AppEvent::ApplicationReopened {
                has_visible_windows,
            });
            let _ = runtime.apply_effects(&effects, &mut core, &mut renderer, button.as_deref());
        }
        _ => {}
    });
}

fn run_redraw_cycle(
    reason: RedrawReason,
    runtime: &mut RuntimeAdapters,
    core: &mut StatletCore,
    renderer: &mut Renderer,
    button: Option<&objc2_app_kit::NSStatusBarButton>,
) -> Vec<AppEffect> {
    objc2::rc::autoreleasepool(|_| {
        let mut target = LiveRedrawTarget {
            runtime,
            core,
            renderer,
            button,
            effects: Vec::new(),
        };
        execute_redraw_reason(reason, &mut target);
        target.effects
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedrawReason {
    Metrics,
    Preferences,
    Appearance,
    Fonts,
    Screens,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeDecision {
    sample: bool,
    save: bool,
    redraw: bool,
    refresh_fonts: bool,
    invalidate_render_cache: bool,
}

fn decision_for(reason: RedrawReason) -> RuntimeDecision {
    RuntimeDecision {
        sample: reason == RedrawReason::Metrics,
        save: false,
        redraw: true,
        refresh_fonts: reason == RedrawReason::Fonts,
        invalidate_render_cache: reason == RedrawReason::Appearance,
    }
}

#[derive(Default)]
struct VisualEnvironmentState {
    last: Option<VisualEnvironment>,
}

impl VisualEnvironmentState {
    fn record(&mut self, current: VisualEnvironment) -> bool {
        if self.last == Some(current) {
            return false;
        }
        self.last = Some(current);
        true
    }
}

trait RuntimeRedrawTarget {
    fn poll_due(&mut self);
    fn coalesce_redraw_effects_owned_by_cycle(&mut self);
    fn refresh_fonts_and_invalidate(&mut self);
    fn invalidate_render_cache(&mut self);
    fn preferences_surface_exists(&self) -> bool;
    fn redraw_indicator_surfaces(&mut self, include_previews: bool);
}

fn coalesce_redraw_effects_owned_by_cycle(effects: &mut Vec<AppEffect>) {
    effects.retain(|effect| !matches!(effect, AppEffect::RedrawIndicator));
}

fn execute_redraw_reason(reason: RedrawReason, target: &mut impl RuntimeRedrawTarget) {
    let decision = decision_for(reason);
    if decision.sample {
        target.poll_due();
    }
    if decision.refresh_fonts {
        target.refresh_fonts_and_invalidate();
    }
    if decision.invalidate_render_cache {
        target.invalidate_render_cache();
    }
    if decision.redraw {
        let include_previews = target.preferences_surface_exists();
        target.redraw_indicator_surfaces(include_previews);
        if decision.sample {
            target.coalesce_redraw_effects_owned_by_cycle();
        }
    }
}

struct LiveRedrawTarget<'a> {
    runtime: &'a mut RuntimeAdapters,
    core: &'a mut StatletCore,
    renderer: &'a mut Renderer,
    button: Option<&'a objc2_app_kit::NSStatusBarButton>,
    effects: Vec<AppEffect>,
}

impl RuntimeRedrawTarget for LiveRedrawTarget<'_> {
    fn poll_due(&mut self) {
        self.effects
            .extend(self.runtime.samplers.poll_due(self.core));
    }

    fn coalesce_redraw_effects_owned_by_cycle(&mut self) {
        coalesce_redraw_effects_owned_by_cycle(&mut self.effects);
    }

    fn refresh_fonts_and_invalidate(&mut self) {
        self.renderer.refresh_fonts();
    }

    fn invalidate_render_cache(&mut self) {
        self.renderer.invalidate();
    }

    fn preferences_surface_exists(&self) -> bool {
        self.runtime
            .windows
            .as_ref()
            .is_some_and(WindowManager::has_preferences_surface)
    }

    fn redraw_indicator_surfaces(&mut self, include_previews: bool) {
        self.runtime.redraw_indicator_surfaces(
            self.core,
            self.renderer,
            self.button,
            include_previews,
        );
    }
}

fn set_next_wakeup(control_flow: &mut ControlFlow, samplers: &RuntimeSamplers) {
    *control_flow = ControlFlow::WaitUntil(Instant::now() + samplers.next_wakeup_in());
}

struct RuntimeAdapters {
    preferences_store: PreferencesStore,
    history_store: HistoryStore,
    history: History,
    windows: Option<WindowManager>,
    samplers: RuntimeSamplers,
    mole: RuntimeMole,
    notifications: Option<NotificationManager>,
    visual_environment_observer: Option<VisualEnvironmentObserver>,
    visual_environment: VisualEnvironmentState,
    review_space_item: MenuItem,
}

impl RuntimeAdapters {
    fn new(
        preferences_store: PreferencesStore,
        history_store: HistoryStore,
        history: History,
        review_space_item: MenuItem,
        proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
        metrics_interval: MetricsRefreshInterval,
    ) -> Self {
        Self {
            preferences_store,
            history_store,
            history,
            windows: None,
            samplers: RuntimeSamplers::new(metrics_interval),
            mole: RuntimeMole::new(proxy),
            notifications: None,
            visual_environment_observer: None,
            visual_environment: VisualEnvironmentState::default(),
            review_space_item,
        }
    }

    fn initialize_native(
        &mut self,
        marker: MainThreadMarker,
        proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) {
        self.windows = Some(WindowManager::new(marker, proxy.clone()));
        self.notifications = NotificationManager::new(marker, proxy.clone());
        let mut observer = VisualEnvironmentObserver::new(marker, proxy);
        observer.rebind_status_button(button);
        self.visual_environment_observer = Some(observer);
    }

    fn rebind_status_button(&mut self, button: Option<&objc2_app_kit::NSStatusBarButton>) {
        if let Some(observer) = &mut self.visual_environment_observer {
            observer.rebind_status_button(button);
        }
    }

    fn apply_effects(
        &mut self,
        effects: &[AppEffect],
        core: &mut StatletCore,
        renderer: &mut Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) -> bool {
        let mut should_quit = false;
        let mut pending = effects.iter().cloned().collect::<VecDeque<_>>();
        while let Some(effect) = pending.pop_front() {
            match effect {
                AppEffect::RedrawIndicator => {
                    pending.extend(run_redraw_cycle(
                        RedrawReason::Preferences,
                        self,
                        core,
                        renderer,
                        button,
                    ));
                }
                AppEffect::SetMetricsSamplingInterval(interval) => {
                    self.samplers.reschedule_metrics(interval);
                }
                AppEffect::ShowWindow(kind) => {
                    if let Some(windows) = &mut self.windows {
                        windows.show(kind, core.state(), &self.history);
                    }
                    if kind == statlet::core::WindowKind::Preferences {
                        pending.extend(run_redraw_cycle(
                            RedrawReason::Preferences,
                            self,
                            core,
                            renderer,
                            button,
                        ));
                    }
                }
                AppEffect::SavePreferences(preferences) => {
                    pending.extend(save_preferences(&self.preferences_store, preferences, core));
                    if let Some(windows) = &self.windows {
                        windows.update_state(core.state());
                    }
                }
                AppEffect::SetDiskSamplingEnabled(enabled) => {
                    self.samplers.set_disk_sampling_enabled(enabled);
                    if !enabled {
                        self.mole.cancel();
                    }
                }
                AppEffect::DiskPressureAlert(observation) => {
                    if let Some(notifications) = &self.notifications {
                        notifications.deliver_disk_pressure(observation);
                    }
                }
                AppEffect::RequestNotificationAuthorization => {
                    if let Some(notifications) = &self.notifications {
                        notifications.request_authorization();
                    }
                }
                AppEffect::CheckMoleCompatibility => {
                    if let Err(error) = self.mole.check() {
                        eprintln!("Statlet could not start Mole compatibility check: {error}");
                        pending.extend(
                            core.handle(AppEvent::MoleStatusObserved(MoleStatus::Unavailable)),
                        );
                    }
                }
                AppEffect::LaunchMoleInTerminal => {
                    if let Err(error) = self.mole.launch_in_terminal() {
                        eprintln!("Statlet could not open Mole in Terminal: {error}");
                    }
                }
                AppEffect::RecordHistory(kind) => {
                    match self.history_store.record(kind, SystemTime::now()) {
                        Ok(history) => {
                            self.history = history;
                            if let Some(windows) = &self.windows {
                                windows.update_history(&self.history);
                            }
                        }
                        Err(error) => eprintln!("Statlet could not record history: {error}"),
                    }
                }
                AppEffect::ClearHistory => match self.history_store.clear() {
                    Ok(history) => {
                        self.history = history;
                        if let Some(windows) = &self.windows {
                            windows.update_history(&self.history);
                        }
                    }
                    Err(error) => eprintln!("Statlet could not clear history: {error}"),
                },
                AppEffect::Quit => should_quit = true,
            }
        }
        self.review_space_item
            .set_enabled(core.state().preferences.mole_integration_enabled);
        if let Some(windows) = &self.windows {
            windows.update_state(core.state());
        }
        should_quit
    }

    fn redraw_indicator_surfaces(
        &mut self,
        core: &StatletCore,
        renderer: &mut Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
        include_previews: bool,
    ) {
        let marker = MainThreadMarker::new().expect("indicator redraws run on the main thread");
        let environment = VisualEnvironment::current(button, marker);
        self.visual_environment.record(environment);
        let preferences = &core.state().preferences.indicator;
        let status_scene =
            compose_indicator(&core.state().status, preferences, environment.appearance);
        let status_layout = button
            .map(|button| renderer.apply_status(button, &status_scene, &preferences.typography));

        if !include_previews {
            return;
        }
        let Some(windows) = &self.windows else {
            return;
        };
        if !windows.has_preferences_surface() {
            return;
        }

        let light_scene = compose_indicator(
            &core.state().status,
            preferences,
            IndicatorAppearance::Light,
        );
        let dark_scene =
            compose_indicator(&core.state().status, preferences, IndicatorAppearance::Dark);
        let preview_plan = environment.preview_plan();
        let light_appearance = preview_appearance(preview_plan.light_appearance);
        let dark_appearance = preview_appearance(preview_plan.dark_appearance);
        let light = renderer.render(
            RenderSlot::PreviewLight,
            &light_scene,
            &preferences.typography,
            &light_appearance,
        );
        let dark = renderer.render(
            RenderSlot::PreviewDark,
            &dark_scene,
            &preferences.typography,
            &dark_appearance,
        );
        let font_fallback = light.font.used_fallback.then(|| IndicatorFontFallback {
            requested_family: light.font.requested_family.clone(),
            resolved_family: light.font.resolved_family.clone(),
        });
        let light_colors = resolved_scene_srgb_colors(&light_scene, &light_appearance);
        let dark_colors = resolved_scene_srgb_colors(&dark_scene, &dark_appearance);
        let contrast_warnings = preview_contrast_warnings(&light_colors, &dark_colors);
        windows.update_indicator_surfaces(IndicatorSurfaceUpdate {
            previews: PreviewImages {
                light: light.image,
                dark: dark.image,
            },
            font_fallback,
            contrast_warnings,
            summaries: PreviewSummaries {
                light: preview_accessibility_summary(
                    &light_scene,
                    &light_colors,
                    IndicatorAppearance::Light,
                ),
                dark: preview_accessibility_summary(
                    &dark_scene,
                    &dark_colors,
                    IndicatorAppearance::Dark,
                ),
            },
            layout: IndicatorLayoutDiagnostics {
                status: status_layout,
                light: light.layout,
                dark: dark.layout,
            },
            environment,
        });
    }
}

fn preview_appearance(name: PreviewAppearanceName) -> objc2::rc::Retained<NSAppearance> {
    let name = unsafe {
        match name {
            PreviewAppearanceName::Aqua => NSAppearanceNameAqua,
            PreviewAppearanceName::DarkAqua => NSAppearanceNameDarkAqua,
            PreviewAppearanceName::HighContrastAqua => {
                NSAppearanceNameAccessibilityHighContrastAqua
            }
            PreviewAppearanceName::HighContrastDarkAqua => {
                NSAppearanceNameAccessibilityHighContrastDarkAqua
            }
        }
    };
    NSAppearance::appearanceNamed(name).expect("named preview appearance is available on macOS")
}

fn preview_contrast_warnings(
    light_colors: &[[f64; 3]],
    dark_colors: &[[f64; 3]],
) -> PreviewContrastWarnings {
    PreviewContrastWarnings {
        light: has_low_text_contrast(light_colors, PreviewBackground::Light),
        dark: has_low_text_contrast(dark_colors, PreviewBackground::Dark),
    }
}

fn save_preferences(
    store: &PreferencesStore,
    preferences: Preferences,
    core: &mut StatletCore,
) -> Vec<AppEffect> {
    let result = match store.save(preferences.clone()) {
        Ok(()) => PreferencesSaveResult::Saved,
        Err(error) => {
            eprintln!("Statlet could not save preferences: {error}");
            PreferencesSaveResult::Failed
        }
    };
    core.handle(AppEvent::PreferencesSaveFinished(result))
}

struct RuntimeMole {
    detector: MoleDetector,
    installation: Option<MoleInstallation>,
    generation: u64,
    check_in_flight: bool,
    proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
}

impl RuntimeMole {
    fn new(proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>) -> Self {
        Self {
            detector: MoleDetector::system(),
            installation: None,
            generation: 0,
            check_in_flight: false,
            proxy,
        }
    }

    fn check(&mut self) -> std::io::Result<()> {
        if self.check_in_flight {
            return Ok(());
        }
        let detector = self.detector.clone();
        self.installation = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let proxy = self.proxy.clone();
        self.check_in_flight = true;
        match thread::Builder::new()
            .name("statlet-mole-check".to_owned())
            .spawn(move || {
                let detection = detector.detect();
                let _ = proxy.send_event(RuntimeEvent::MoleDetected {
                    generation,
                    detection,
                });
            }) {
            Ok(_) => Ok(()),
            Err(error) => {
                self.check_in_flight = false;
                Err(error)
            }
        }
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.check_in_flight = false;
        self.installation = None;
    }

    fn apply_detection(&mut self, generation: u64, detection: MoleDetection) -> Option<MoleStatus> {
        if generation != self.generation {
            return None;
        }
        self.check_in_flight = false;
        self.installation = detection.installation;
        Some(detection.status)
    }

    fn launch_in_terminal(&self) -> std::io::Result<()> {
        let installation = self.installation.clone().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "compatible Mole is unavailable",
            )
        })?;
        thread::Builder::new()
            .name("statlet-mole-launch".to_owned())
            .spawn(move || match installation.terminal_launch_plan().launch() {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    eprintln!("Statlet could not open Mole: osascript exited with {status}")
                }
                Err(error) => eprintln!("Statlet could not open Mole in Terminal: {error}"),
            })?;
        Ok(())
    }
}

struct RuntimeSamplers {
    metrics: MacSampler,
    metrics_schedule: MetricsSamplingSchedule,
    disk: StartupVolumeSampler,
    disk_schedule: DiskSamplingSchedule,
    clock: ContinuousClock,
}

impl RuntimeSamplers {
    fn new(metrics_interval: MetricsRefreshInterval) -> Self {
        let mut metrics = MacSampler::new();
        metrics.prime_cpu();
        let clock = ContinuousClock::new().expect("initialize the macOS continuous clock");
        Self {
            metrics,
            metrics_schedule: MetricsSamplingSchedule::new_due_now(clock.now(), metrics_interval),
            disk: StartupVolumeSampler::new(),
            disk_schedule: DiskSamplingSchedule::new(),
            clock,
        }
    }

    fn set_disk_sampling_enabled(&mut self, enabled: bool) {
        self.disk_schedule.set_enabled(enabled, self.clock.now());
    }

    fn poll_due(&mut self, core: &mut StatletCore) -> Vec<AppEffect> {
        let now = self.clock.now();
        if self.metrics_schedule.take_due(now) {
            if let Some(snapshot) = self.metrics.sample() {
                core.handle(AppEvent::MetricsSample(snapshot));
            }
        }
        if self.disk_schedule.take_due(now) {
            match self.disk.sample(now) {
                Ok(observation) => core.handle(AppEvent::DiskObserved(observation)),
                Err(error) => {
                    eprintln!("Statlet could not sample the startup volume: {error:?}");
                    core.handle(AppEvent::DiskMonitoringFailed)
                }
            }
        } else {
            Vec::new()
        }
    }

    fn reschedule_metrics(&mut self, interval: MetricsRefreshInterval) {
        self.metrics_schedule.reschedule(self.clock.now(), interval);
    }

    fn next_wakeup_in(&self) -> Duration {
        let now = self.clock.now();
        let metrics = self.metrics_schedule.remaining(now);
        self.disk_schedule
            .remaining(now)
            .map_or(metrics, |disk| disk.min(metrics))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use statlet::core::{AppEffect, Preferences, PreferencesSaveStatus};
    use statlet::history::HistoryEventKind;
    use statlet::indicator::{IndicatorRun, IndicatorScene, SegmentColor, SemanticColor};
    use statlet::indicator_preferences::{MetricsRefreshInterval, SrgbColor};
    use tempfile::tempdir;

    use super::{
        coalesce_redraw_effects_owned_by_cycle, decision_for, execute_redraw_reason,
        preview_contrast_warnings, resolved_scene_srgb_colors, save_preferences, PreferencesStore,
        PreviewContrastWarnings, RedrawReason, RuntimeDecision, RuntimeRedrawTarget,
        RuntimeSamplers, StatletCore, VisualEnvironment, VisualEnvironmentState,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedAction {
        Sample,
        Save,
        RefreshFontsAndInvalidate,
        InvalidateRenderCache,
        RedrawStatus,
        RedrawPreviewLight,
        RedrawPreviewDark,
    }

    #[test]
    fn repeated_visual_environment_notification_does_not_request_another_redraw() {
        let standard = VisualEnvironment {
            appearance: statlet::indicator_preferences::IndicatorAppearance::Light,
            increase_contrast: false,
            differentiate_without_color: false,
            reduce_transparency: false,
        };
        let mut state = VisualEnvironmentState::default();

        assert!(state.record(standard));
        assert!(!state.record(standard));
        assert!(state.record(VisualEnvironment {
            increase_contrast: true,
            ..standard
        }));
    }

    struct FakeRedrawTarget {
        preferences_surface_exists: bool,
        actions: Vec<RecordedAction>,
        pending_poll_effects: Vec<AppEffect>,
        effects: Vec<AppEffect>,
        applied_effects: Vec<AppEffect>,
    }

    impl FakeRedrawTarget {
        fn new(preferences_surface_exists: bool) -> Self {
            Self {
                preferences_surface_exists,
                actions: Vec::new(),
                pending_poll_effects: Vec::new(),
                effects: Vec::new(),
                applied_effects: Vec::new(),
            }
        }

        fn with_poll_result(
            preferences_surface_exists: bool,
            pending_poll_effects: Vec<AppEffect>,
        ) -> Self {
            Self {
                pending_poll_effects,
                ..Self::new(preferences_surface_exists)
            }
        }

        fn apply_polled_effects(&mut self) {
            for effect in std::mem::take(&mut self.effects) {
                match effect {
                    AppEffect::RedrawIndicator => {
                        execute_redraw_reason(RedrawReason::Preferences, self)
                    }
                    AppEffect::SavePreferences(_) => self.actions.push(RecordedAction::Save),
                    effect => self.applied_effects.push(effect),
                }
            }
        }
    }

    impl RuntimeRedrawTarget for FakeRedrawTarget {
        fn poll_due(&mut self) {
            self.actions.push(RecordedAction::Sample);
            self.effects.append(&mut self.pending_poll_effects);
        }

        fn coalesce_redraw_effects_owned_by_cycle(&mut self) {
            coalesce_redraw_effects_owned_by_cycle(&mut self.effects);
        }

        fn refresh_fonts_and_invalidate(&mut self) {
            self.actions.push(RecordedAction::RefreshFontsAndInvalidate);
        }

        fn invalidate_render_cache(&mut self) {
            self.actions.push(RecordedAction::InvalidateRenderCache);
        }

        fn preferences_surface_exists(&self) -> bool {
            self.preferences_surface_exists
        }

        fn redraw_indicator_surfaces(&mut self, include_previews: bool) {
            self.actions.push(RecordedAction::RedrawStatus);
            if include_previews {
                self.actions.push(RecordedAction::RedrawPreviewLight);
                self.actions.push(RecordedAction::RedrawPreviewDark);
            }
        }
    }

    #[test]
    fn visual_reasons_select_redraw_without_sampling_or_saving() {
        for reason in [
            RedrawReason::Appearance,
            RedrawReason::Fonts,
            RedrawReason::Screens,
        ] {
            assert_eq!(
                decision_for(reason),
                RuntimeDecision {
                    sample: false,
                    save: false,
                    redraw: true,
                    refresh_fonts: reason == RedrawReason::Fonts,
                    invalidate_render_cache: reason == RedrawReason::Appearance,
                }
            );
        }
    }

    #[test]
    fn visual_environment_changes_invalidate_semantic_color_paint_without_side_effects() {
        let decision = decision_for(RedrawReason::Appearance);

        assert!(decision.invalidate_render_cache);
        assert!(!decision.sample);
        assert!(!decision.save);
        assert!(decision.redraw);

        let mut target = FakeRedrawTarget::new(false);
        execute_redraw_reason(RedrawReason::Appearance, &mut target);
        assert_eq!(
            target.actions,
            vec![
                RecordedAction::InvalidateRenderCache,
                RecordedAction::RedrawStatus,
            ]
        );
    }

    #[test]
    fn metric_due_may_sample_then_redraw_but_preference_redraw_never_samples() {
        let mut metrics = FakeRedrawTarget::new(false);
        execute_redraw_reason(RedrawReason::Metrics, &mut metrics);
        assert_eq!(
            metrics.actions,
            vec![RecordedAction::Sample, RecordedAction::RedrawStatus]
        );

        let mut preferences = FakeRedrawTarget::new(false);
        execute_redraw_reason(RedrawReason::Preferences, &mut preferences);
        assert_eq!(preferences.actions, vec![RecordedAction::RedrawStatus]);
    }

    #[test]
    fn polling_effect_application_coalesces_the_owned_redraw_and_preserves_other_effects() {
        for preferences_surface_exists in [false, true] {
            let mut target = FakeRedrawTarget::with_poll_result(
                preferences_surface_exists,
                vec![
                    AppEffect::RedrawIndicator,
                    AppEffect::RecordHistory(HistoryEventKind::MonitoringFailed),
                ],
            );

            execute_redraw_reason(RedrawReason::Metrics, &mut target);
            target.apply_polled_effects();

            assert_eq!(
                target
                    .actions
                    .iter()
                    .filter(|action| **action == RecordedAction::Sample)
                    .count(),
                1
            );
            assert_eq!(
                target
                    .actions
                    .iter()
                    .filter(|action| **action == RecordedAction::RedrawStatus)
                    .count(),
                1
            );
            assert_eq!(
                target
                    .actions
                    .iter()
                    .filter(|action| {
                        matches!(
                            action,
                            RecordedAction::RedrawPreviewLight | RecordedAction::RedrawPreviewDark
                        )
                    })
                    .count(),
                2 * usize::from(preferences_surface_exists)
            );
            assert!(!target.actions.contains(&RecordedAction::Save));
            assert_eq!(
                target.applied_effects,
                vec![AppEffect::RecordHistory(HistoryEventKind::MonitoringFailed)]
            );
        }
    }

    #[test]
    fn font_event_refreshes_and_invalidates_before_redrawing() {
        let mut target = FakeRedrawTarget::new(true);

        execute_redraw_reason(RedrawReason::Fonts, &mut target);

        assert_eq!(
            target.actions,
            vec![
                RecordedAction::RefreshFontsAndInvalidate,
                RecordedAction::RedrawStatus,
                RecordedAction::RedrawPreviewLight,
                RecordedAction::RedrawPreviewDark,
            ]
        );
    }

    #[test]
    fn previews_are_skipped_until_the_preferences_surface_exists() {
        let mut absent = FakeRedrawTarget::new(false);
        execute_redraw_reason(RedrawReason::Appearance, &mut absent);
        assert_eq!(
            absent.actions,
            vec![
                RecordedAction::InvalidateRenderCache,
                RecordedAction::RedrawStatus,
            ]
        );

        let mut created = FakeRedrawTarget::new(true);
        execute_redraw_reason(RedrawReason::Appearance, &mut created);
        assert_eq!(
            created.actions,
            vec![
                RecordedAction::InvalidateRenderCache,
                RecordedAction::RedrawStatus,
                RecordedAction::RedrawPreviewLight,
                RecordedAction::RedrawPreviewDark,
            ]
        );
    }

    #[test]
    fn preview_contrast_metadata_flags_custom_text_below_the_threshold() {
        let gray = SegmentColor::Srgb(SrgbColor::parse_hex("#777777").unwrap());
        let scene = IndicatorScene {
            top: vec![IndicatorRun {
                text: "C 42%".into(),
                color: gray,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".into(),
                color: gray,
            }],
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%".into(),
        };

        let warnings = warnings_for_named_previews(&scene, &scene);

        assert!(warnings.light);
        assert!(warnings.dark);
    }

    #[test]
    fn preview_contrast_metadata_evaluates_semantic_colors_in_both_appearances() {
        let warning = SegmentColor::Semantic(SemanticColor::Warning);
        let scene = IndicatorScene {
            top: vec![IndicatorRun {
                text: "C 42%".into(),
                color: warning,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".into(),
                color: warning,
            }],
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%".into(),
        };

        let warnings = warnings_for_named_previews(&scene, &scene);

        assert!(warnings.light);
        assert!(!warnings.dark);
    }

    fn warnings_for_named_previews(
        light_scene: &IndicatorScene,
        dark_scene: &IndicatorScene,
    ) -> PreviewContrastWarnings {
        let light = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua }).unwrap();
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).unwrap();
        let light_colors = resolved_scene_srgb_colors(light_scene, &light);
        let dark_colors = resolved_scene_srgb_colors(dark_scene, &dark);
        preview_contrast_warnings(&light_colors, &dark_colors)
    }

    struct DeadlineTarget<'a> {
        samplers: &'a mut RuntimeSamplers,
    }

    impl RuntimeRedrawTarget for DeadlineTarget<'_> {
        fn poll_due(&mut self) {
            panic!("visual redraw must not poll samplers");
        }

        fn coalesce_redraw_effects_owned_by_cycle(&mut self) {}

        fn refresh_fonts_and_invalidate(&mut self) {}

        fn invalidate_render_cache(&mut self) {}

        fn preferences_surface_exists(&self) -> bool {
            false
        }

        fn redraw_indicator_surfaces(&mut self, _include_previews: bool) {
            let _ = &self.samplers;
        }
    }

    #[test]
    fn redraw_paths_do_not_change_metric_or_disk_deadlines() {
        let mut samplers = RuntimeSamplers::new(MetricsRefreshInterval::try_from(30).unwrap());
        let now = samplers.clock.now();
        samplers.reschedule_metrics(MetricsRefreshInterval::try_from(30).unwrap());
        samplers.disk_schedule.set_enabled(true, now);
        assert!(samplers.disk_schedule.take_due(now));
        let metrics_deadline = samplers.metrics_schedule.remaining(Duration::ZERO);
        let disk_deadline = samplers.disk_schedule.remaining(Duration::ZERO);

        for reason in [
            RedrawReason::Preferences,
            RedrawReason::Appearance,
            RedrawReason::Fonts,
            RedrawReason::Screens,
        ] {
            execute_redraw_reason(
                reason,
                &mut DeadlineTarget {
                    samplers: &mut samplers,
                },
            );
            assert_eq!(
                samplers.metrics_schedule.remaining(Duration::ZERO),
                metrics_deadline
            );
            assert_eq!(
                samplers.disk_schedule.remaining(Duration::ZERO),
                disk_deadline
            );
        }
    }

    #[test]
    fn runtime_constructor_uses_the_loaded_metrics_interval() {
        let mut samplers = RuntimeSamplers::new(MetricsRefreshInterval::try_from(60).unwrap());
        let now = samplers.clock.now();

        assert!(samplers.metrics_schedule.take_due(now));
        assert_eq!(
            samplers.metrics_schedule.remaining(now),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn next_wakeup_uses_disk_when_it_is_due_before_metrics() {
        let mut samplers = RuntimeSamplers::new(MetricsRefreshInterval::try_from(60).unwrap());
        samplers.reschedule_metrics(MetricsRefreshInterval::try_from(60).unwrap());
        samplers.set_disk_sampling_enabled(true);

        assert_eq!(samplers.next_wakeup_in(), Duration::ZERO);
    }

    #[test]
    fn runtime_reschedules_metrics_without_moving_the_disk_deadline() {
        let mut samplers = RuntimeSamplers::new(MetricsRefreshInterval::try_from(2).unwrap());
        let before_reschedule = samplers.clock.now();
        samplers.disk_schedule.set_enabled(true, before_reschedule);
        assert!(samplers.disk_schedule.take_due(before_reschedule));
        let disk_deadline = samplers.disk_schedule.remaining(Duration::ZERO);

        samplers.reschedule_metrics(MetricsRefreshInterval::try_from(30).unwrap());

        let after_reschedule = samplers.clock.now();
        let metrics_deadline = samplers.metrics_schedule.remaining(Duration::ZERO);
        assert!(
            metrics_deadline >= before_reschedule + Duration::from_secs(30)
                && metrics_deadline <= after_reschedule + Duration::from_secs(30)
        );
        assert_eq!(
            samplers.disk_schedule.remaining(Duration::ZERO),
            disk_deadline
        );
    }

    #[test]
    fn runtime_save_reports_real_failure_and_later_success_to_the_reducer() {
        let directory = tempdir().unwrap();
        let blocked_parent = directory.path().join("not-a-directory");
        fs::write(&blocked_parent, "blocking file").unwrap();
        let failing_store = PreferencesStore::new(blocked_parent.join("preferences.json"));
        let successful_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let preferences = Preferences::default();
        let mut core = StatletCore::new();

        assert!(save_preferences(&failing_store, preferences.clone(), &mut core).is_empty());
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Failed
        );

        assert!(save_preferences(&successful_store, preferences.clone(), &mut core).is_empty());
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
        assert_eq!(successful_store.load(), preferences);
    }
}
