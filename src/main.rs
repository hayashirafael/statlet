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
    AppEffect, AppEvent, MetricPngAssetMutation, MetricPngImportResult, MetricPngRemovalResult,
    Preferences, PreferencesSaveResult, StatletCore,
};
use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};
use statlet::disk::DiskSamplingSchedule;
use statlet::history::{History, HistoryStore};
use statlet::icon_assets::{IconAssetStore, PngAssetTransaction, PreparedPngAsset};
use statlet::indicator::{
    compose_indicator, has_low_text_contrast, preview_accessibility_summary_with_fallbacks,
    preview_visible_summary, resolve_identifier_fallbacks, PreviewBackground,
};
use statlet::indicator_preferences::{IndicatorAppearance, MetricKind, MetricsRefreshInterval};
use statlet::metrics_schedule::MetricsSamplingSchedule;
use statlet::mole::{MoleDetection, MoleDetector, MoleInstallation, MoleStatus};
use statlet::preferences::{PreferencesCommitState, PreferencesSaveError, PreferencesStore};
use statlet::runtime_schedule::{RedrawRequest, RuntimeSchedule};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use macos::environment::{PreviewAppearanceName, VisualEnvironment, VisualEnvironmentObserver};
use macos::gpu::MacGpuSampler;
use macos::notifications::NotificationManager;
use macos::renderer::{resolved_scene_srgb_colors, PreviewImages, RenderSlot, Renderer};
use macos::sampler::MacSampler;
use macos::windows::{
    IndicatorFontFallback, IndicatorLayoutDiagnostics, IndicatorSurfaceUpdate,
    PreviewContrastWarnings, PreviewSummaries, WindowManager,
};
use macos::RuntimeEvent;
use statlet::stats::{
    ProcessSampleCancellation, ProcessSampleCompletion, SystemUsageModel,
    SystemUsageRenderCoalescer, SystemUsageSamplingCoordinator, SystemUsageSamplingPorts,
    SystemUsageSamplingSchedule, SystemUsageSection,
};

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
    let system_usage_item = MenuItem::new("Uso do sistema…", true, None);
    let system_usage_id: MenuId = system_usage_item.id().clone();
    let quit = MenuItem::new("Sair", true, None);
    let quit_id: MenuId = quit.id().clone();
    let menu = Menu::new();
    menu.append(&system_usage_item).expect("build menu");
    menu.append(&review_space_item).expect("build menu");
    menu.append(&preferences_item).expect("build menu");
    menu.append(&history_item).expect("build menu");
    menu.append(&quit).expect("build menu");

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let runtime_event = if event.id == review_space_id {
            Some(RuntimeEvent::App(AppEvent::ReviewSpace))
        } else if event.id == system_usage_id {
            Some(RuntimeEvent::App(AppEvent::OpenSystemUsage))
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
                RuntimeEvent::MetricPngPrepared {
                    metric,
                    generation,
                    result,
                } => runtime.finish_png_preparation(&mut core, metric, generation, result),
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
                RuntimeEvent::ProcessesSampled {
                    generation,
                    visibility_generation,
                    outcome,
                } => {
                    let live_visible = runtime.system_usage_visible();
                    let live_visibility_generation = runtime.system_usage_visibility_generation();
                    let interaction_active = runtime.system_usage_process_interaction_active();
                    runtime.samplers.record_processes(
                        generation,
                        visibility_generation,
                        live_visibility_generation,
                        outcome,
                        live_visible,
                        interaction_active,
                    );
                    Vec::new()
                }
                RuntimeEvent::SystemUsageVisibilityChanged(visible) => {
                    runtime.samplers.set_system_usage_visible(visible);
                    Vec::new()
                }
                RuntimeEvent::SystemUsageSectionSelectedByUser(section) => {
                    runtime.request_system_usage_summary_focus(section);
                    core.handle(AppEvent::SelectSystemUsageSection(section))
                }
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
    event_proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
    png_import_generations: [u64; 2],
    prepared_png_imports: [Option<PreparedPngAsset>; 2],
    system_usage_rendering: SystemUsageRenderCoalescer,
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
            samplers: RuntimeSamplers::new(metrics_interval, Some(proxy.clone())),
            mole: RuntimeMole::new(proxy.clone()),
            notifications: None,
            visual_environment_observer: None,
            visual_environment: VisualEnvironmentState::default(),
            schedule: RuntimeSchedule::new(),
            review_space_item,
            system_usage_rendering: SystemUsageRenderCoalescer::new(),
            event_proxy: proxy,
            png_import_generations: [0; 2],
            prepared_png_imports: [None, None],
        }
    }

    fn finish_png_preparation(
        &mut self,
        core: &mut StatletCore,
        metric: MetricKind,
        generation: u64,
        result: Result<PreparedPngAsset, String>,
    ) -> Vec<AppEffect> {
        let index = metric_index(metric);
        if !png_import_generation_is_current(&self.png_import_generations, metric, generation) {
            return Vec::new();
        }
        match result {
            Ok(prepared) => {
                let metadata = prepared.metadata().clone();
                self.prepared_png_imports[index] = Some(prepared);
                core.handle(AppEvent::MetricPngImportFinished {
                    metric,
                    result: MetricPngImportResult::Imported(metadata),
                })
            }
            Err(message) => core.handle(AppEvent::MetricPngImportFinished {
                metric,
                result: MetricPngImportResult::Failed(message),
            }),
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
        let system_usage_deadline = self
            .samplers
            .system_usage_schedule
            .remaining(now)
            .map(|remaining| now + remaining);
        self.schedule
            .next_deadline(metrics_deadline, system_usage_deadline, disk_deadline)
            .saturating_sub(now)
    }

    fn process_due(
        &mut self,
        core: &mut StatletCore,
        renderer: &mut Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) -> bool {
        let now = self.samplers.clock.now();
        let system_usage_visible = self.system_usage_visible();
        let process_interaction_active = self.system_usage_process_interaction_active();
        let system_usage_visibility_generation = self.system_usage_visibility_generation();
        let poll = self.samplers.poll_due(
            core,
            system_usage_visible,
            process_interaction_active,
            system_usage_visibility_generation,
        );
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

    fn update_system_usage_window(&mut self, section: SystemUsageSection) {
        if let Some(windows) = &self.windows {
            if windows.system_usage_visible() {
                let view_model = self.samplers.system_usage_view_model(section);
                if self.system_usage_rendering.take_changed(&view_model) {
                    windows.update_system_usage(&view_model);
                }
            }
        }
    }

    fn system_usage_visible(&self) -> bool {
        self.windows
            .as_ref()
            .is_some_and(WindowManager::system_usage_visible)
    }

    fn system_usage_process_interaction_active(&self) -> bool {
        self.windows
            .as_ref()
            .is_some_and(WindowManager::system_usage_process_interaction_active)
    }

    fn system_usage_visibility_generation(&self) -> u64 {
        self.windows
            .as_ref()
            .map_or(0, WindowManager::system_usage_visibility_generation)
    }

    fn request_system_usage_summary_focus(&self, section: SystemUsageSection) {
        if let Some(windows) = &self.windows {
            windows.request_system_usage_summary_focus(section);
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
                    if kind == statlet::core::WindowKind::SystemUsage {
                        self.system_usage_rendering.reset();
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
                    let generation =
                        next_png_import_generation(&mut self.png_import_generations, metric);
                    if let Err(error) = spawn_png_preparation_with(
                        self.icon_asset_store.clone(),
                        metric,
                        source,
                        generation,
                        {
                            let proxy = self.event_proxy.clone();
                            move |event| {
                                let _ = proxy.send_event(event);
                            }
                        },
                    ) {
                        pending.extend(core.handle(AppEvent::MetricPngImportFinished {
                            metric,
                            result: MetricPngImportResult::Failed(format!(
                                "Não foi possível iniciar o processamento do PNG: {error}"
                            )),
                        }));
                    }
                }
                AppEffect::CancelMetricPngImport(metric) => {
                    invalidate_png_import(
                        &mut self.png_import_generations,
                        &mut self.prepared_png_imports,
                        metric,
                    );
                }
                AppEffect::RemoveMetricPngAsset(metric) => {
                    pending.extend(core.handle(AppEvent::MetricPngRemovalFinished {
                        metric,
                        result: MetricPngRemovalResult::Removed,
                    }));
                }
                AppEffect::PersistMetricPngChange {
                    metric,
                    mutation,
                    previous,
                    preferences,
                } => {
                    let transaction = match mutation {
                        MetricPngAssetMutation::Replace => self.prepared_png_imports
                            [metric_index(metric)]
                        .take()
                        .ok_or_else(|| {
                            "O PNG preparado não está mais disponível; a escolha anterior foi restaurada."
                                .to_owned()
                        })
                        .and_then(|prepared| {
                            self.icon_asset_store
                                .begin_replace(prepared)
                                .map_err(|error| error.user_message().to_owned())
                        }),
                        MetricPngAssetMutation::Remove => self
                            .icon_asset_store
                            .begin_remove(metric)
                            .map_err(|error| error.user_message().to_owned()),
                    };
                    match transaction {
                        Ok(transaction) => pending.extend(persist_metric_png_change(
                            &self.preferences_store,
                            &mut self.schedule,
                            self.samplers.clock.now(),
                            core,
                            metric,
                            previous,
                            preferences,
                            transaction,
                        )),
                        Err(message) => {
                            pending.extend(core.handle(AppEvent::MetricPngPersistenceFailed {
                                metric,
                                previous,
                                message,
                            }))
                        }
                    }
                }
                AppEffect::Quit => should_quit = true,
            }
        }
        self.review_space_item
            .set_enabled(core.state().preferences.mole_integration_enabled);
        if let Some(windows) = &self.windows {
            windows.update_state(core.state());
        }
        self.update_system_usage_window(core.state().system_usage_section);
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
        let (light_resolved_scene, light_fallbacks) =
            resolve_identifier_fallbacks(&light_scene, light.identifier_resolved);
        let (dark_resolved_scene, dark_fallbacks) =
            resolve_identifier_fallbacks(&dark_scene, dark.identifier_resolved);
        let light_colors = resolved_scene_srgb_colors(&light_resolved_scene, &light_appearance);
        let dark_colors = resolved_scene_srgb_colors(&dark_resolved_scene, &dark_appearance);
        let contrast_warnings = preview_contrast_warnings(&light_colors, &dark_colors);
        windows.update_indicator_surfaces(IndicatorSurfaceUpdate {
            previews: PreviewImages {
                light: light.image,
                dark: dark.image,
            },
            font_fallback,
            contrast_warnings,
            summaries: PreviewSummaries {
                light_visible: preview_visible_summary(
                    &light_resolved_scene,
                    IndicatorAppearance::Light,
                    light_fallbacks,
                ),
                dark_visible: preview_visible_summary(
                    &dark_resolved_scene,
                    IndicatorAppearance::Dark,
                    dark_fallbacks,
                ),
                light: preview_accessibility_summary_with_fallbacks(
                    &light_resolved_scene,
                    &light_colors,
                    IndicatorAppearance::Light,
                    light_fallbacks,
                ),
                dark: preview_accessibility_summary_with_fallbacks(
                    &dark_resolved_scene,
                    &dark_colors,
                    IndicatorAppearance::Dark,
                    dark_fallbacks,
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

trait PreferencesPersistence {
    fn save(&self, preferences: Preferences) -> Result<(), PreferencesPersistenceError>;
}

#[derive(Debug)]
struct PreferencesPersistenceError {
    commit_state: PreferencesCommitState,
    message: String,
}

impl From<PreferencesSaveError> for PreferencesPersistenceError {
    fn from(error: PreferencesSaveError) -> Self {
        Self {
            commit_state: error.commit_state(),
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for PreferencesPersistenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl PreferencesPersistence for PreferencesStore {
    fn save(&self, preferences: Preferences) -> Result<(), PreferencesPersistenceError> {
        PreferencesStore::save(self, preferences).map_err(Into::into)
    }
}

fn save_preferences(
    store: &impl PreferencesPersistence,
    preferences: Preferences,
    core: &mut StatletCore,
) -> bool {
    let (result, succeeded) = match store.save(preferences) {
        Ok(()) => (PreferencesSaveResult::Saved, true),
        Err(error) if error.commit_state == PreferencesCommitState::Committed => {
            eprintln!("Statlet committed preferences with a durability warning: {error}");
            (PreferencesSaveResult::Failed, false)
        }
        Err(error) => {
            eprintln!("Statlet could not save preferences: {error}");
            (PreferencesSaveResult::Failed, false)
        }
    };
    let effects = core.handle(AppEvent::PreferencesSaveFinished(result));
    debug_assert!(effects.is_empty());
    succeeded
}

fn metric_index(metric: MetricKind) -> usize {
    match metric {
        MetricKind::Cpu => 0,
        MetricKind::Ram => 1,
    }
}

fn next_png_import_generation(generations: &mut [u64; 2], metric: MetricKind) -> u64 {
    let generation = &mut generations[metric_index(metric)];
    *generation = generation.wrapping_add(1);
    *generation
}

fn png_import_generation_is_current(
    generations: &[u64; 2],
    metric: MetricKind,
    generation: u64,
) -> bool {
    generations[metric_index(metric)] == generation
}

fn invalidate_png_import(
    generations: &mut [u64; 2],
    prepared_imports: &mut [Option<PreparedPngAsset>; 2],
    metric: MetricKind,
) {
    next_png_import_generation(generations, metric);
    prepared_imports[metric_index(metric)] = None;
}

fn spawn_png_preparation_with(
    store: IconAssetStore,
    metric: MetricKind,
    source: std::path::PathBuf,
    generation: u64,
    deliver: impl FnOnce(RuntimeEvent) + Send + 'static,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name(format!(
            "statlet-png-{}",
            match metric {
                MetricKind::Cpu => "cpu",
                MetricKind::Ram => "ram",
            }
        ))
        .spawn(move || {
            let result = store
                .prepare_file(metric, &source)
                .map_err(|error| error.user_message().to_owned());
            deliver(RuntimeEvent::MetricPngPrepared {
                metric,
                generation,
                result,
            });
        })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_metric_png_change<T: MetricPngTransaction>(
    store: &impl PreferencesPersistence,
    schedule: &mut RuntimeSchedule<Preferences>,
    now: Duration,
    core: &mut StatletCore,
    metric: MetricKind,
    previous: statlet::indicator_preferences::MetricIdentifierPreferences,
    preferences: Preferences,
    transaction: T,
) -> Vec<AppEffect> {
    schedule.queue_save(now, preferences.clone());
    schedule.request_save_now(now);
    debug_assert_eq!(schedule.due_save(now), Some(preferences.clone()));
    match store.save(preferences.clone()) {
        Ok(()) => {
            let cleanup_error = transaction.commit().err();
            schedule.finish_save(&preferences, true);
            let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                PreferencesSaveResult::Saved,
            ));
            if let Some(error) = cleanup_error {
                effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                    metric,
                    message: format!(
                        "O PNG foi salvo, mas a limpeza segura da transação falhou: {error}"
                    ),
                }));
            }
            effects
        }
        Err(error) if error.commit_state == PreferencesCommitState::Committed => {
            eprintln!(
                "Statlet committed preferences for a PNG change with a durability warning: {error}"
            );
            let cleanup_error = transaction.commit().err();
            schedule.finish_save(&preferences, false);
            let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                PreferencesSaveResult::Failed,
            ));
            effects.extend(core.handle(AppEvent::MetricPngDurabilityWarning {
                metric,
                message: format!(
                    "As preferências e o PNG foram aplicados, mas a confirmação de durabilidade falhou: {error}"
                ),
            }));
            if let Some(cleanup_error) = cleanup_error {
                effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                    metric,
                    message: format!(
                        "O PNG foi aplicado, mas a limpeza segura da transação falhou: {cleanup_error}"
                    ),
                }));
            }
            effects
        }
        Err(error) => {
            eprintln!("Statlet could not save preferences for a PNG change: {error}");
            schedule.finish_save(&preferences, false);
            let rollback = transaction.rollback();
            let message = match rollback {
                Ok(()) => {
                    "Não foi possível salvar as preferências; a escolha de PNG anterior foi restaurada."
                        .to_owned()
                }
                Err(rollback_error) => format!(
                    "Não foi possível salvar as preferências nem restaurar o PNG anterior: {rollback_error}"
                ),
            };
            core.handle(AppEvent::MetricPngPersistenceFailed {
                metric,
                previous,
                message,
            })
        }
    }
}

trait MetricPngTransaction {
    fn commit(self) -> Result<(), String>;
    fn rollback(self) -> Result<(), String>;
}

impl MetricPngTransaction for PngAssetTransaction {
    fn commit(self) -> Result<(), String> {
        PngAssetTransaction::commit(self).map_err(|error| error.user_message().to_owned())
    }

    fn rollback(self) -> Result<(), String> {
        PngAssetTransaction::rollback(self).map_err(|error| error.user_message().to_owned())
    }
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
    gpu: MacGpuSampler,
    system_usage: SystemUsageModel,
    system_usage_schedule: SystemUsageSamplingSchedule,
    system_usage_sampling: SystemUsageSamplingCoordinator,
    process_proxy: Option<tao::event_loop::EventLoopProxy<RuntimeEvent>>,
}

struct RuntimePoll {
    effects: Vec<AppEffect>,
    metrics_ticked: bool,
}

impl RuntimeSamplers {
    fn new(
        metrics_interval: MetricsRefreshInterval,
        process_proxy: Option<tao::event_loop::EventLoopProxy<RuntimeEvent>>,
    ) -> Self {
        let mut metrics = MacSampler::new();
        metrics.prime_cpu();
        let clock = ContinuousClock::new().expect("initialize the macOS continuous clock");
        Self {
            metrics,
            metrics_schedule: MetricsSamplingSchedule::new_due_now(clock.now(), metrics_interval),
            disk: StartupVolumeSampler::new(),
            disk_schedule: DiskSamplingSchedule::new(),
            clock,
            gpu: MacGpuSampler::new(),
            system_usage: SystemUsageModel::new(),
            system_usage_schedule: SystemUsageSamplingSchedule::new(),
            system_usage_sampling: SystemUsageSamplingCoordinator::new(),
            process_proxy,
        }
    }

    fn set_disk_sampling_enabled(&mut self, enabled: bool) {
        self.disk_schedule.set_enabled(enabled, self.clock.now());
    }

    fn set_system_usage_visible(&mut self, visible: bool) {
        let now = self.clock.now();
        self.system_usage.set_visible(visible);
        self.system_usage_schedule.set_visible(visible, now);
        self.system_usage_sampling.set_visible(now, visible);
    }

    fn poll_due(
        &mut self,
        core: &mut StatletCore,
        system_usage_visible: bool,
        process_interaction_active: bool,
        system_usage_visibility_generation: u64,
    ) -> RuntimePoll {
        let now = self.clock.now();
        self.system_usage.set_visible(system_usage_visible);
        self.system_usage_schedule
            .set_visible(system_usage_visible, now);
        self.system_usage_sampling
            .set_visible(now, system_usage_visible);
        self.system_usage
            .apply_deferred_processes(now, process_interaction_active);
        let metrics_ticked = self.metrics_schedule.take_due(now);
        let system_usage_ticked = self.system_usage_schedule.take_due(now);
        if metrics_ticked || system_usage_ticked {
            match self.metrics.sample() {
                Some(snapshot) => {
                    if metrics_ticked {
                        core.handle(AppEvent::MetricsSample(snapshot.compact));
                    }
                    if system_usage_ticked {
                        self.system_usage.record_memory(now, Ok(snapshot.memory));
                    }
                }
                None if system_usage_ticked => self.system_usage.record_memory(now, Err(())),
                None => {}
            }
        }
        if system_usage_ticked {
            let mut ports = RuntimeSystemUsagePorts {
                gpu: &mut self.gpu,
                process_proxy: self.process_proxy.as_ref(),
                visibility_generation: system_usage_visibility_generation,
            };
            if let Some(gpu) =
                self.system_usage_sampling
                    .collect_if_visible(now, system_usage_visible, &mut ports)
            {
                self.system_usage.record_gpu(now, gpu);
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

    fn system_usage_view_model(
        &self,
        section: SystemUsageSection,
    ) -> statlet::stats::SystemUsageViewModel {
        self.system_usage.view_model(section)
    }

    fn record_processes(
        &mut self,
        generation: u64,
        request_visibility_generation: u64,
        live_visibility_generation: u64,
        outcome: statlet::stats::ProcessSampleOutcome,
        live_visible: bool,
        interaction_active: bool,
    ) {
        let now = self.clock.now();
        self.system_usage.set_visible(live_visible);
        self.system_usage_sampling.record_processes_if_current(
            ProcessSampleCompletion {
                observed_at: now,
                live_visible,
                interaction_active,
                request_visibility_generation,
                live_visibility_generation,
                generation,
                outcome,
            },
            &mut self.system_usage,
        );
    }
}

struct RuntimeSystemUsagePorts<'a> {
    gpu: &'a mut MacGpuSampler,
    process_proxy: Option<&'a tao::event_loop::EventLoopProxy<RuntimeEvent>>,
    visibility_generation: u64,
}

impl SystemUsageSamplingPorts for RuntimeSystemUsagePorts<'_> {
    fn sample_gpu(&mut self) -> statlet::stats::GpuSampleOutcome {
        self.gpu.sample()
    }

    fn request_process_sample(
        &mut self,
        generation: u64,
        cancellation: ProcessSampleCancellation,
    ) -> bool {
        let Some(process_proxy) = self.process_proxy else {
            return false;
        };
        let proxy = process_proxy.clone();
        let visibility_generation = self.visibility_generation;
        thread::Builder::new()
            .name("statlet-process-sample".to_owned())
            .spawn(move || {
                let outcome = MacSampler::sample_processes(&cancellation);
                let _ = proxy.send_event(RuntimeEvent::ProcessesSampled {
                    generation,
                    visibility_generation,
                    outcome,
                });
            })
            .map(|_| true)
            .unwrap_or_else(|_| {
                process_proxy
                    .send_event(RuntimeEvent::ProcessesSampled {
                        generation,
                        visibility_generation,
                        outcome: statlet::stats::ProcessSampleOutcome::Failed,
                    })
                    .is_ok()
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::io::Cursor;
    use std::sync::mpsc;
    use std::time::Duration;

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use statlet::core::{AppEvent, IndicatorPreferenceChange, Preferences, PreferencesSaveStatus};
    use statlet::indicator::{IndicatorRun, IndicatorScene, SegmentColor, SemanticColor};
    use statlet::indicator_preferences::{MetricsRefreshInterval, SrgbColor};
    use statlet::preferences::PreferencesCommitState;
    use tempfile::tempdir;

    use super::{
        apply_persistence_intent, preview_contrast_warnings, resolved_scene_srgb_colors,
        save_preferences, spawn_png_preparation_with, visual_environment_redraw_request, AppEffect,
        IconAssetStore, MetricKind, PreferencesStore, PreviewContrastWarnings, RuntimeEvent,
        RuntimeSamplers, StatletCore, VisualEnvironment, VisualEnvironmentState,
    };

    #[test]
    fn png_decode_resize_and_reencode_run_on_a_worker_thread() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("large.png");
        let image = RgbaImage::from_pixel(128, 64, Rgba([0x22, 0x88, 0xCC, 0xFF]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        fs::write(&source, bytes.into_inner()).unwrap();
        let store = IconAssetStore::new(directory.path().join("icons"));
        let caller = std::thread::current().id();
        let (sender, receiver) = mpsc::channel();

        spawn_png_preparation_with(store, MetricKind::Cpu, source, 7, move |event| {
            sender.send((std::thread::current().id(), event)).unwrap();
        })
        .unwrap();

        let (worker, event) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(worker, caller);
        assert!(matches!(
            event,
            RuntimeEvent::MetricPngPrepared {
                metric: MetricKind::Cpu,
                generation: 7,
                result: Ok(_),
            }
        ));
    }

    #[test]
    fn canceling_a_png_import_invalidates_its_runtime_generation() {
        let mut generations = [0, 0];
        let mut prepared = [None, None];
        let stale_generation = super::next_png_import_generation(&mut generations, MetricKind::Cpu);

        super::invalidate_png_import(&mut generations, &mut prepared, MetricKind::Cpu);

        assert!(!super::png_import_generation_is_current(
            &generations,
            MetricKind::Cpu,
            stale_generation,
        ));
    }

    #[test]
    fn png_asset_and_preferences_roll_back_together_when_save_fails() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let png = |color| {
            let image = RgbaImage::from_pixel(12, 12, Rgba(color));
            let mut bytes = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, ImageFormat::Png)
                .unwrap();
            bytes.into_inner()
        };
        asset_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let original_asset = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        let prepared = asset_store
            .prepare_bytes(
                MetricKind::Cpu,
                "replacement.png",
                &png([0xAA, 0xBB, 0xCC, 0xFF]),
            )
            .unwrap();
        let metadata = prepared.metadata().clone();
        let transaction = asset_store.begin_replace(prepared).unwrap();
        let blocker = directory.path().join("not-a-directory");
        fs::write(&blocker, b"block preference parent").unwrap();
        let preferences_store = PreferencesStore::new(blocker.join("preferences.json"));
        let mut core = StatletCore::new();
        let previous = core.state().preferences.indicator.identifiers.cpu.clone();
        let effect = core
            .handle(AppEvent::MetricPngImportFinished {
                metric: MetricKind::Cpu,
                result: statlet::core::MetricPngImportResult::Imported(metadata),
            })
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistMetricPngChange {
                    metric,
                    previous,
                    preferences,
                    ..
                } => Some((metric, previous, preferences)),
                _ => None,
            })
            .unwrap();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_metric_png_change(
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            effect.0,
            effect.1,
            effect.2,
            transaction,
        );

        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            original_asset
        );
        assert_eq!(core.state().preferences.indicator.identifiers.cpu, previous);
        assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
    }

    #[derive(Debug)]
    struct FaultInjectedTransaction {
        commit_error: Option<String>,
        rollback_error: Option<String>,
    }

    struct PostRenameFaultStore {
        inner: PreferencesStore,
    }

    struct PostRenameOnceStore {
        inner: PreferencesStore,
        fail_after_next_save: Cell<bool>,
    }

    impl super::PreferencesPersistence for PostRenameFaultStore {
        fn save(&self, preferences: Preferences) -> Result<(), super::PreferencesPersistenceError> {
            self.inner
                .save(preferences)
                .map_err(super::PreferencesPersistenceError::from)?;
            Err(super::PreferencesPersistenceError {
                commit_state: PreferencesCommitState::Committed,
                message: "fault injected after preferences rename".into(),
            })
        }
    }

    impl super::PreferencesPersistence for PostRenameOnceStore {
        fn save(&self, preferences: Preferences) -> Result<(), super::PreferencesPersistenceError> {
            self.inner
                .save(preferences)
                .map_err(super::PreferencesPersistenceError::from)?;
            if self.fail_after_next_save.replace(false) {
                Err(super::PreferencesPersistenceError {
                    commit_state: PreferencesCommitState::Committed,
                    message: "fault injected after preferences rename".into(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn post_rename_preferences_failure_keeps_json_asset_and_runtime_state_aligned() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let png = |color| {
            let image = RgbaImage::from_pixel(12, 12, Rgba(color));
            let mut bytes = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, ImageFormat::Png)
                .unwrap();
            bytes.into_inner()
        };
        asset_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let prepared = asset_store
            .prepare_bytes(
                MetricKind::Cpu,
                "replacement.png",
                &png([0xAA, 0xBB, 0xCC, 0xFF]),
            )
            .unwrap();
        let metadata = prepared.metadata().clone();
        let transaction = asset_store.begin_replace(prepared).unwrap();
        let installed_asset = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        preferences_store.save(Preferences::default()).unwrap();
        let fault_store = PostRenameFaultStore {
            inner: preferences_store,
        };
        let mut core = StatletCore::new();
        let (metric, previous, preferences) = core
            .handle(AppEvent::MetricPngImportFinished {
                metric: MetricKind::Cpu,
                result: statlet::core::MetricPngImportResult::Imported(metadata),
            })
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistMetricPngChange {
                    metric,
                    previous,
                    preferences,
                    ..
                } => Some((metric, previous, preferences)),
                _ => None,
            })
            .unwrap();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_metric_png_change(
            &fault_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            metric,
            previous,
            preferences.clone(),
            transaction,
        );

        assert!(effects.is_empty());
        assert_eq!(fault_store.inner.load(), preferences);
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            installed_asset
        );
        assert_eq!(core.state().preferences, fault_store.inner.load());
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Failed
        );
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Cpu)
            .unwrap()
            .contains("fault injected after preferences rename"));
        assert_eq!(schedule.pending_save(), Some(&preferences));
    }

    #[test]
    fn successful_retry_clears_only_the_resolved_png_durability_warning() {
        let directory = tempdir().unwrap();
        let store = PostRenameOnceStore {
            inner: PreferencesStore::new(directory.path().join("preferences.json")),
            fail_after_next_save: Cell::new(true),
        };
        let mut core = StatletCore::new();
        let previous = core.state().preferences.indicator.identifiers.cpu.clone();
        let metadata =
            statlet::indicator_preferences::PngIconMetadata::new("custom-cpu.png", 24, 12, 812)
                .unwrap();
        let preferences = core
            .handle(AppEvent::MetricPngImportFinished {
                metric: MetricKind::Cpu,
                result: statlet::core::MetricPngImportResult::Imported(metadata),
            })
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistMetricPngChange { preferences, .. } => Some(preferences),
                _ => None,
            })
            .unwrap();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        super::persist_metric_png_change(
            &store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            MetricKind::Cpu,
            previous,
            preferences.clone(),
            FaultInjectedTransaction {
                commit_error: None,
                rollback_error: None,
            },
        );
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Cpu)
            .unwrap()
            .contains("confirmação de durabilidade"));

        assert_eq!(
            core.handle(AppEvent::RetrySavePreferences),
            vec![AppEffect::FlushPreferences(preferences.clone())]
        );
        let succeeded = save_preferences(&store, preferences.clone(), &mut core);
        schedule.finish_save(&preferences, succeeded);

        assert!(succeeded);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
        assert_eq!(schedule.pending_save(), None);
        assert_eq!(core.state().indicator_icon_error(MetricKind::Cpu), None);
    }

    #[test]
    fn successful_retry_preserves_an_independent_png_cleanup_error() {
        let directory = tempdir().unwrap();
        let store = PostRenameOnceStore {
            inner: PreferencesStore::new(directory.path().join("preferences.json")),
            fail_after_next_save: Cell::new(true),
        };
        let mut core = StatletCore::new();
        let previous = core.state().preferences.indicator.identifiers.ram.clone();
        let preferences = core.state().preferences.clone();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        super::persist_metric_png_change(
            &store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            MetricKind::Ram,
            previous,
            preferences.clone(),
            FaultInjectedTransaction {
                commit_error: Some("fault injected during backup cleanup".into()),
                rollback_error: None,
            },
        );
        let succeeded = save_preferences(&store, preferences.clone(), &mut core);
        schedule.finish_save(&preferences, succeeded);

        let error = core.state().indicator_icon_error(MetricKind::Ram).unwrap();
        assert!(succeeded);
        assert_eq!(schedule.pending_save(), None);
        assert!(error.contains("fault injected during backup cleanup"));
        assert!(!error.contains("confirmação de durabilidade"));
    }

    impl super::MetricPngTransaction for FaultInjectedTransaction {
        fn commit(self) -> Result<(), String> {
            self.commit_error.map_or(Ok(()), Err)
        }

        fn rollback(self) -> Result<(), String> {
            self.rollback_error.map_or(Ok(()), Err)
        }
    }

    #[test]
    fn committed_preferences_surface_transaction_cleanup_failure() {
        let directory = tempdir().unwrap();
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let mut core = StatletCore::new();
        let previous = core.state().preferences.indicator.identifiers.cpu.clone();
        let preferences = core.state().preferences.clone();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_metric_png_change(
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            MetricKind::Cpu,
            previous,
            preferences,
            FaultInjectedTransaction {
                commit_error: Some("fault injected during backup cleanup".into()),
                rollback_error: None,
            },
        );

        assert!(effects.is_empty());
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Cpu)
            .unwrap()
            .contains("fault injected during backup cleanup"));
    }

    #[test]
    fn failed_preferences_save_surfaces_rollback_failure() {
        let directory = tempdir().unwrap();
        let blocker = directory.path().join("not-a-directory");
        fs::write(&blocker, b"block preference parent").unwrap();
        let preferences_store = PreferencesStore::new(blocker.join("preferences.json"));
        let mut core = StatletCore::new();
        let previous = core.state().preferences.indicator.identifiers.ram.clone();
        let preferences = core.state().preferences.clone();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_metric_png_change(
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            MetricKind::Ram,
            previous,
            preferences,
            FaultInjectedTransaction {
                commit_error: None,
                rollback_error: Some("fault injected while restoring previous.png".into()),
            },
        );

        assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Failed
        );
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Ram)
            .unwrap()
            .contains("fault injected while restoring previous.png"));
    }

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
        let mut samplers =
            RuntimeSamplers::new(MetricsRefreshInterval::try_from(60).unwrap(), None);
        let now = samplers.clock.now();

        assert!(samplers.metrics_schedule.take_due(now));
        assert_eq!(
            samplers.metrics_schedule.remaining(now),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn runtime_reschedules_metrics_without_moving_the_disk_deadline() {
        let mut samplers = RuntimeSamplers::new(MetricsRefreshInterval::try_from(2).unwrap(), None);
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
