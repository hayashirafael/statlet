//! Statlet macOS runtime.
//!
//! Event-loop structure derived and modified from featherbar commit 90ab504,
//! licensed under Apache-2.0.

mod macos;

use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAccessibilityHighContrastAqua,
    NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua,
};
use statlet::core::{
    AppEffect, AppEvent, MetricPngImportResult, MetricPngRemovalResult, Preferences,
    PreferencesSaveResult, StatletCore,
};
use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};
use statlet::disk::DiskSamplingSchedule;
use statlet::history::{History, HistoryStore};
use statlet::icon_assets::IconAssetStore;
use statlet::indicator::{
    compose_indicator, has_low_text_contrast, preview_accessibility_summary, PreviewBackground,
};
use statlet::indicator_preferences::{IndicatorAppearance, MetricsRefreshInterval};
use statlet::metrics_schedule::MetricsSamplingSchedule;
use statlet::mole::{MoleDetection, MoleDetector, MoleInstallation, MoleStatus};
use statlet::preferences::PreferencesStore;
use statlet::runtime_schedule::{RedrawRequest, RuntimeSchedule};
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
    let icon_asset_store = IconAssetStore::for_current_user()
        .expect("resolve the current user's indicator icon directory");
    let history = history_store.load();
    let initial_preferences = preferences_store.load();
    let initial_metrics_interval = initial_preferences.indicator.refresh_interval;
    let (mut core, startup_effects) = StatletCore::with_preferences(initial_preferences);
    let mut runtime = RuntimeAdapters::new(
        preferences_store,
        icon_asset_store,
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
            let _ = runtime.process_due(&mut core, &mut renderer, button.as_deref());
            set_next_wakeup(control_flow, &runtime);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            if button.is_none() {
                let marker = MainThreadMarker::new().expect("main-thread event loop");
                button = macos::renderer::status_button(marker);
                runtime.rebind_status_button(button.as_deref());
            }
            let _ = runtime.process_due(&mut core, &mut renderer, button.as_deref());
            set_next_wakeup(control_flow, &runtime);
        }
        Event::UserEvent(runtime_event) => {
            let effects = match runtime_event {
                RuntimeEvent::App(app_event) => core.handle(app_event),
                RuntimeEvent::VisualEnvironmentChanged => {
                    let marker = MainThreadMarker::new().expect("visual events run on main thread");
                    if let Some(request) = visual_environment_redraw_request(
                        runtime.refresh_visual_environment(button.as_deref(), marker),
                    ) {
                        runtime.request_redraw(request);
                    }
                    Vec::new()
                }
                RuntimeEvent::FontSetChanged => {
                    runtime.request_redraw(RedrawRequest::fonts());
                    Vec::new()
                }
                RuntimeEvent::ScreenParametersChanged => {
                    let marker = MainThreadMarker::new().expect("screen events run on main thread");
                    let request = visual_environment_redraw_request(
                        runtime.refresh_visual_environment(button.as_deref(), marker),
                    )
                    .unwrap_or_else(RedrawRequest::paint);
                    runtime.request_redraw(request);
                    Vec::new()
                }
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
                set_next_wakeup(control_flow, &runtime);
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
            set_next_wakeup(control_flow, &runtime);
        }
        _ => {}
    });
}

struct VisualEnvironmentState<A> {
    last: Option<VisualEnvironment>,
    status_appearance: Option<A>,
    status_appearance_identity: Option<String>,
}

impl<A> Default for VisualEnvironmentState<A> {
    fn default() -> Self {
        Self {
            last: None,
            status_appearance: None,
            status_appearance_identity: None,
        }
    }
}

impl<A> VisualEnvironmentState<A> {
    fn refresh_with<I: Into<String>>(
        &mut self,
        read: impl FnOnce() -> (VisualEnvironment, A, I),
    ) -> bool {
        let (current, status_appearance, status_appearance_identity) = read();
        self.record(current, status_appearance, status_appearance_identity)
    }

    fn record(
        &mut self,
        current: VisualEnvironment,
        status_appearance: A,
        status_appearance_identity: impl Into<String>,
    ) -> bool {
        let status_appearance_identity = status_appearance_identity.into();
        let changed = self.last != Some(current)
            || self.status_appearance_identity.as_ref() != Some(&status_appearance_identity);
        self.last = Some(current);
        self.status_appearance = Some(status_appearance);
        self.status_appearance_identity = Some(status_appearance_identity);
        changed
    }

    fn current(&self) -> (VisualEnvironment, &A) {
        (
            self.last
                .expect("visual environment is captured during native initialization"),
            self.status_appearance
                .as_ref()
                .expect("status appearance is captured during native initialization"),
        )
    }
}

fn visual_environment_redraw_request(changed: bool) -> Option<RedrawRequest> {
    changed.then(RedrawRequest::semantic_colors)
}

fn set_next_wakeup(control_flow: &mut ControlFlow, runtime: &RuntimeAdapters) {
    *control_flow = ControlFlow::WaitUntil(Instant::now() + runtime.next_wakeup_in());
}

fn apply_persistence_intent(
    schedule: &mut RuntimeSchedule<Preferences>,
    now: Duration,
    effect: &AppEffect,
) -> Option<Preferences> {
    match effect {
        AppEffect::QueuePreferencesSave(preferences) => {
            schedule.queue_save(now, preferences.clone());
            None
        }
        AppEffect::FlushPreferences(preferences) if schedule.pending_save().is_some() => {
            schedule.queue_save(now, preferences.clone());
            schedule.request_save_now(now);
            schedule.due_save(now)
        }
        AppEffect::FlushPreferences(_) => None,
        _ => None,
    }
}

struct RuntimeAdapters {
    preferences_store: PreferencesStore,
    icon_asset_store: IconAssetStore,
    history_store: HistoryStore,
    history: History,
    windows: Option<WindowManager>,
    samplers: RuntimeSamplers,
    mole: RuntimeMole,
    notifications: Option<NotificationManager>,
    visual_environment_observer: Option<VisualEnvironmentObserver>,
    visual_environment: VisualEnvironmentState<Retained<NSAppearance>>,
    schedule: RuntimeSchedule<Preferences>,
    review_space_item: MenuItem,
}

impl RuntimeAdapters {
    fn new(
        preferences_store: PreferencesStore,
        icon_asset_store: IconAssetStore,
        history_store: HistoryStore,
        history: History,
        review_space_item: MenuItem,
        proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
        metrics_interval: MetricsRefreshInterval,
    ) -> Self {
        Self {
            preferences_store,
            icon_asset_store,
            history_store,
            history,
            windows: None,
            samplers: RuntimeSamplers::new(metrics_interval),
            mole: RuntimeMole::new(proxy),
            notifications: None,
            visual_environment_observer: None,
            visual_environment: VisualEnvironmentState::default(),
            schedule: RuntimeSchedule::new(),
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
        self.visual_environment
            .refresh_with(|| VisualEnvironment::current(button, marker));
    }

    fn rebind_status_button(&mut self, button: Option<&objc2_app_kit::NSStatusBarButton>) {
        if let Some(observer) = &mut self.visual_environment_observer {
            observer.rebind_status_button(button);
        }
    }

    fn refresh_visual_environment(
        &mut self,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
        marker: MainThreadMarker,
    ) -> bool {
        self.visual_environment
            .refresh_with(|| VisualEnvironment::current(button, marker))
    }

    fn request_redraw(&mut self, request: RedrawRequest) {
        self.schedule
            .request_redraw(self.samplers.clock.now(), request);
    }

    fn next_wakeup_in(&self) -> Duration {
        let now = self.samplers.clock.now();
        let metrics_deadline = now + self.samplers.metrics_schedule.remaining(now);
        let disk_deadline = self
            .samplers
            .disk_schedule
            .remaining(now)
            .map(|remaining| now + remaining);
        self.schedule
            .next_deadline(metrics_deadline, disk_deadline)
            .saturating_sub(now)
    }

    fn process_due(
        &mut self,
        core: &mut StatletCore,
        renderer: &mut Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) -> bool {
        let now = self.samplers.clock.now();
        let poll = self.samplers.poll_due(core);
        if poll.metrics_ticked {
            self.schedule
                .request_redraw_now(now, RedrawRequest::paint());
        }
        if self.apply_effects(&poll.effects, core, renderer, button) {
            return true;
        }

        if let Some(request) = self.schedule.take_due_redraw(now) {
            objc2::rc::autoreleasepool(|_| {
                if request.refresh_fonts {
                    renderer.refresh_fonts();
                } else if request.invalidate_semantic_colors {
                    renderer.invalidate_semantic_colors();
                }
                let include_previews = self
                    .windows
                    .as_ref()
                    .is_some_and(WindowManager::has_preferences_surface);
                self.redraw_indicator_surfaces(core, renderer, button, include_previews);
            });
        }
        self.save_due_preferences(now, core);
        false
    }

    fn save_due_preferences(&mut self, now: Duration, core: &mut StatletCore) {
        let Some(preferences) = self.schedule.due_save(now) else {
            return;
        };
        let succeeded = save_preferences(&self.preferences_store, preferences.clone(), core);
        self.schedule.finish_save(&preferences, succeeded);
        if let Some(windows) = &self.windows {
            windows.update_state(core.state());
        }
    }

    fn apply_effects(
        &mut self,
        effects: &[AppEffect],
        core: &mut StatletCore,
        _renderer: &mut Renderer,
        _button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) -> bool {
        let mut should_quit = false;
        let mut pending = effects.iter().cloned().collect::<VecDeque<_>>();
        while let Some(effect) = pending.pop_front() {
            match effect {
                AppEffect::RequestIndicatorRedraw => {
                    self.request_redraw(RedrawRequest::paint());
                }
                AppEffect::SetMetricsSamplingInterval(interval) => {
                    self.samplers.reschedule_metrics(interval);
                }
                AppEffect::ShowWindow(kind) => {
                    if let Some(windows) = &mut self.windows {
                        windows.show(kind, core.state(), &self.history);
                    }
                    if kind == statlet::core::WindowKind::Preferences {
                        self.request_redraw(RedrawRequest::paint());
                    }
                }
                AppEffect::QueuePreferencesSave(preferences) => {
                    let effect = AppEffect::QueuePreferencesSave(preferences);
                    let due = apply_persistence_intent(
                        &mut self.schedule,
                        self.samplers.clock.now(),
                        &effect,
                    );
                    debug_assert!(due.is_none());
                }
                AppEffect::FlushPreferences(preferences) => {
                    let now = self.samplers.clock.now();
                    let effect = AppEffect::FlushPreferences(preferences);
                    if let Some(preferences) =
                        apply_persistence_intent(&mut self.schedule, now, &effect)
                    {
                        let succeeded =
                            save_preferences(&self.preferences_store, preferences.clone(), core);
                        self.schedule.finish_save(&preferences, succeeded);
                        if let Some(windows) = &self.windows {
                            windows.update_state(core.state());
                        }
                    }
                }
                AppEffect::ReleasePreferencesWindow => {
                    if let Some(windows) = &mut self.windows {
                        windows.release_preferences();
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
                AppEffect::ImportMetricPng { metric, source } => {
                    let result = match self.icon_asset_store.import_file(metric, &source) {
                        Ok(metadata) => MetricPngImportResult::Imported(metadata),
                        Err(error) => {
                            MetricPngImportResult::Failed(error.user_message().to_owned())
                        }
                    };
                    pending
                        .extend(core.handle(AppEvent::MetricPngImportFinished { metric, result }));
                }
                AppEffect::RemoveMetricPngAsset(metric) => {
                    let result = match self.icon_asset_store.remove(metric) {
                        Ok(()) => MetricPngRemovalResult::Removed,
                        Err(error) => {
                            MetricPngRemovalResult::Failed(error.user_message().to_owned())
                        }
                    };
                    pending
                        .extend(core.handle(AppEvent::MetricPngRemovalFinished { metric, result }));
                }
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
        let (environment, status_appearance) = self.visual_environment.current();
        let preferences = &core.state().preferences.indicator;
        let status_scene =
            compose_indicator(&core.state().status, preferences, environment.appearance);
        let status_layout = button.map(|button| {
            renderer.apply_status(
                button,
                &status_scene,
                &preferences.typography,
                status_appearance,
            )
        });

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
) -> bool {
    let (result, succeeded) = match store.save(preferences) {
        Ok(()) => (PreferencesSaveResult::Saved, true),
        Err(error) => {
            eprintln!("Statlet could not save preferences: {error}");
            (PreferencesSaveResult::Failed, false)
        }
    };
    let effects = core.handle(AppEvent::PreferencesSaveFinished(result));
    debug_assert!(effects.is_empty());
    succeeded
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

struct RuntimePoll {
    effects: Vec<AppEffect>,
    metrics_ticked: bool,
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

    fn poll_due(&mut self, core: &mut StatletCore) -> RuntimePoll {
        let now = self.clock.now();
        let metrics_ticked = self.metrics_schedule.take_due(now);
        if metrics_ticked {
            if let Some(snapshot) = self.metrics.sample() {
                core.handle(AppEvent::MetricsSample(snapshot));
            }
        }
        let effects = if self.disk_schedule.take_due(now) {
            match self.disk.sample(now) {
                Ok(observation) => core.handle(AppEvent::DiskObserved(observation)),
                Err(error) => {
                    eprintln!("Statlet could not sample the startup volume: {error:?}");
                    core.handle(AppEvent::DiskMonitoringFailed)
                }
            }
        } else {
            Vec::new()
        };
        RuntimePoll {
            effects,
            metrics_ticked,
        }
    }

    fn reschedule_metrics(&mut self, interval: MetricsRefreshInterval) {
        self.metrics_schedule.reschedule(self.clock.now(), interval);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use statlet::core::{AppEvent, IndicatorPreferenceChange, Preferences, PreferencesSaveStatus};
    use statlet::indicator::{IndicatorRun, IndicatorScene, SegmentColor, SemanticColor};
    use statlet::indicator_preferences::{MetricsRefreshInterval, SrgbColor};
    use tempfile::tempdir;

    use super::{
        apply_persistence_intent, preview_contrast_warnings, resolved_scene_srgb_colors,
        save_preferences, visual_environment_redraw_request, PreferencesStore,
        PreviewContrastWarnings, RuntimeSamplers, StatletCore, VisualEnvironment,
        VisualEnvironmentState,
    };

    #[test]
    fn appearance_identity_change_requests_only_a_semantic_color_redraw() {
        let standard = VisualEnvironment {
            appearance: statlet::indicator_preferences::IndicatorAppearance::Light,
            increase_contrast: false,
            differentiate_without_color: false,
            reduce_transparency: false,
        };
        let mut state = VisualEnvironmentState::default();

        assert!(state.record(standard, "first-handle", "standard-aqua"));
        let changed = state.record(standard, "second-handle", "refreshed-aqua");

        assert_eq!(
            visual_environment_redraw_request(changed),
            Some(statlet::runtime_schedule::RedrawRequest::semantic_colors())
        );
        let (_, status_appearance) = state.current();
        assert_eq!(*status_appearance, "second-handle");
    }

    #[test]
    fn equivalent_appearance_identity_replaces_the_handle_without_requesting_redraw() {
        let standard = VisualEnvironment {
            appearance: statlet::indicator_preferences::IndicatorAppearance::Light,
            increase_contrast: false,
            differentiate_without_color: false,
            reduce_transparency: false,
        };
        let mut state = VisualEnvironmentState::default();

        assert!(state.record(standard, "first-handle", "standard-aqua"));
        let changed = state.record(standard, "second-handle", "standard-aqua");

        assert_eq!(visual_environment_redraw_request(changed), None);
        let (_, status_appearance) = state.current();
        assert_eq!(*status_appearance, "second-handle");
    }

    #[test]
    fn visual_environment_change_still_requests_only_a_semantic_color_redraw() {
        let standard = VisualEnvironment {
            appearance: statlet::indicator_preferences::IndicatorAppearance::Light,
            increase_contrast: false,
            differentiate_without_color: false,
            reduce_transparency: false,
        };
        let mut state = VisualEnvironmentState::default();

        assert!(state.record(standard, "first-handle", "standard-aqua"));
        let changed = state.record(
            VisualEnvironment {
                increase_contrast: true,
                ..standard
            },
            "second-handle",
            "standard-aqua",
        );

        assert_eq!(
            visual_environment_redraw_request(changed),
            Some(statlet::runtime_schedule::RedrawRequest::semantic_colors())
        );
    }

    #[test]
    fn retained_visual_environment_is_read_once_per_observer_refresh_not_per_metrics_tick() {
        let reads = std::cell::Cell::new(0);
        let standard = VisualEnvironment {
            appearance: statlet::indicator_preferences::IndicatorAppearance::Light,
            increase_contrast: false,
            differentiate_without_color: false,
            reduce_transparency: false,
        };
        let mut state = VisualEnvironmentState::<&str>::default();

        assert!(state.refresh_with(|| {
            reads.set(reads.get() + 1);
            (standard, "retained-handle", "retained-aqua")
        }));
        for _ in 0..10 {
            let (environment, status_appearance) = state.current();
            assert_eq!(environment, standard);
            assert_eq!(*status_appearance, "retained-handle");
        }
        assert_eq!(reads.get(), 1);

        assert!(!state.refresh_with(|| {
            reads.set(reads.get() + 1);
            (standard, "refreshed-handle", "retained-aqua")
        }));
        let (_, status_appearance) = state.current();
        assert_eq!(*status_appearance, "refreshed-handle");
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn close_and_quit_flush_the_latest_queued_document_immediately() {
        for terminal_event in [AppEvent::PreferencesWindowClosed, AppEvent::Quit] {
            let mut core = StatletCore::new();
            let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
            let update_effects = core.handle(AppEvent::UpdateIndicator(
                IndicatorPreferenceChange::SetLabelsVisible(false),
            ));
            for effect in &update_effects {
                assert_eq!(
                    apply_persistence_intent(&mut schedule, Duration::ZERO, effect),
                    None
                );
            }

            let latest = core.state().preferences.clone();
            let terminal_effects = core.handle(terminal_event.clone());
            let due = terminal_effects.iter().find_map(|effect| {
                apply_persistence_intent(&mut schedule, Duration::from_millis(20), effect)
            });

            assert_eq!(due, Some(latest));
        }
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
            top_identifier: None,
            bottom_identifier: None,
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
            top_identifier: None,
            bottom_identifier: None,
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

        assert!(!save_preferences(
            &failing_store,
            preferences.clone(),
            &mut core
        ));
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Failed
        );

        assert!(save_preferences(
            &successful_store,
            preferences.clone(),
            &mut core
        ));
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
        assert_eq!(successful_store.load(), preferences);
    }
}
