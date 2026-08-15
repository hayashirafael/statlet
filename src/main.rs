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
    AppEffect, AppEvent, GlobalIndicatorResetFailure, GlobalIndicatorUndoFailure,
    GlobalIndicatorUndoFailureStage, MetricPngAssetMutation, MetricPngImportResult,
    MetricPngRemovalResult, Preferences, PreferencesSaveResult, StatletCore,
};
use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};
use statlet::disk::DiskSamplingSchedule;
use statlet::history::{History, HistoryStore};
use statlet::icon_assets::{
    IconAssetStore, IndicatorPngSnapshot, PngAssetTransaction, PreparedPngAsset,
};
use statlet::indicator::{
    compose_indicator, has_low_text_contrast, preview_accessibility_summary_with_fallbacks,
    preview_visible_summary, resolve_identifier_fallbacks, PreviewBackground,
};
use statlet::indicator_preferences::{
    IdentifierPreferences, IndicatorAppearance, IndicatorPreferences, MetricKind,
    MetricsRefreshInterval,
};
use statlet::metrics::MemoryReading;
use statlet::metrics_schedule::MetricsSamplingSchedule;
use statlet::mole::{MoleDetection, MoleDetector, MoleInstallation, MoleStatus};
use statlet::preferences::{PreferencesCommitState, PreferencesSaveError, PreferencesStore};
use statlet::runtime_profile::{RuntimeProfile, StorageOverrides};
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
use statlet::system_usage::{
    ProcessSampleRequest, ProcessStart, SamplingCycle, SurfaceObservation, SystemUsageCause,
    SystemUsagePresentation, SystemUsageSampling, SystemUsageSession, SystemUsageSurface,
};

fn main() {
    let profile = RuntimeProfile::resolve(macos::bundle_profile_metadata())
        .expect("resolve the Statlet bundle runtime profile");
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME is required to resolve Statlet storage");
    let storage = profile
        .storage(
            &home,
            StorageOverrides {
                preferences_path: std::env::var_os("STATLET_PREFERENCES_PATH")
                    .map(std::path::PathBuf::from),
                icon_assets_directory: std::env::var_os("STATLET_ICON_ASSETS_DIR")
                    .map(std::path::PathBuf::from),
            },
        )
        .expect("resolve Statlet storage for the runtime profile");
    let presentation = profile.presentation();

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
    let dev_identity_item = presentation
        .menu_identity()
        .map(|identity| MenuItem::new(identity, false, None));
    if let Some(identity) = &dev_identity_item {
        menu.append(identity)
            .expect("build Dev identity menu header");
    }
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

    let preferences_store = PreferencesStore::new(storage.preferences_path);
    let history_store = HistoryStore::new(storage.history_path);
    let icon_asset_store = IconAssetStore::new(storage.icon_assets_directory);
    let history = history_store.load();
    let initial_preferences = preferences_store.load();
    let initial_metrics_interval = initial_preferences.indicator.refresh_interval;
    let (mut core, startup_effects) = StatletCore::with_preferences(initial_preferences);
    let mut renderer = Renderer::new(icon_asset_store.clone(), presentation.clone());
    let mut runtime = RuntimeAdapters::new(
        RuntimeStores {
            preferences: preferences_store,
            icon_assets: icon_asset_store,
            history: history_store,
        },
        history,
        review_space_item,
        proxy.clone(),
        initial_metrics_interval,
        presentation,
    );
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
                RuntimeEvent::ProcessesSampled(completion) => {
                    runtime.advance_system_usage(SystemUsageCause::ProcessesFinished(completion));
                    Vec::new()
                }
                RuntimeEvent::SystemUsageSurfaceChanged => {
                    runtime.advance_system_usage(SystemUsageCause::SurfaceChanged);
                    Vec::new()
                }
                RuntimeEvent::SystemUsageSectionSelectedByUser(section) => {
                    let effects = core.handle(AppEvent::SelectSystemUsageSection(section));
                    runtime.advance_system_usage(SystemUsageCause::SelectSection(section));
                    effects
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

fn recovered_preferences(core: &StatletCore, completion: &AppEvent) -> Option<Preferences> {
    let mut preferences = core.state().preferences.clone();
    match completion {
        AppEvent::MetricPngPersistenceFailed {
            metric, previous, ..
        } => match metric {
            MetricKind::Cpu => preferences.indicator.identifiers.cpu = previous.clone(),
            MetricKind::Ram => preferences.indicator.identifiers.ram = previous.clone(),
        },
        AppEvent::IdentifierResetPersistenceFailed { previous, .. } => {
            preferences.indicator.identifiers = previous.clone();
        }
        AppEvent::GlobalIndicatorResetPersistenceFailed(failure) => {
            preferences.indicator = failure.previous.clone();
        }
        AppEvent::GlobalIndicatorUndoPersistenceFailed(failure) => {
            preferences.indicator = failure.current.clone();
        }
        _ => return None,
    }
    Some(preferences)
}

struct RecoveryCompletionAttempt {
    effects: Vec<AppEffect>,
    retry: Option<PendingRecoveryCompletion>,
}

fn persist_recovery_completion(
    store: &impl PreferencesPersistence,
    schedule: &mut RuntimeSchedule<Preferences>,
    now: Duration,
    core: &mut StatletCore,
    completion: PendingRecoveryCompletion,
) -> RecoveryCompletionAttempt {
    let Some(restored) = recovered_preferences(core, &completion.event) else {
        return RecoveryCompletionAttempt {
            effects: Vec::new(),
            retry: None,
        };
    };
    schedule.queue_save(now, restored.clone());
    schedule.request_save_now(now);
    match store.save(restored.clone()) {
        Ok(()) => {
            schedule.finish_save(&restored, true);
            let mut effects = core.handle(completion.event);
            effects.extend(core.handle(AppEvent::PreferencesSaveFinished(
                PreferencesSaveResult::Saved,
            )));
            RecoveryCompletionAttempt {
                effects,
                retry: None,
            }
        }
        Err(error) if error.commit_state == PreferencesCommitState::Committed => {
            schedule.finish_save(&restored, false);
            let mut effects = core.handle(completion.event);
            effects.extend(core.handle(AppEvent::PreferencesSaveFinished(
                PreferencesSaveResult::Failed,
            )));
            for metric in completion.metrics {
                effects.extend(core.handle(AppEvent::MetricPngDurabilityWarning {
                    metric,
                    message: format!(
                        "A recuperação foi persistida, mas a confirmação de durabilidade falhou: {error}"
                    ),
                }));
            }
            RecoveryCompletionAttempt {
                effects,
                retry: None,
            }
        }
        Err(error) => {
            schedule.finish_save(&restored, false);
            let mut effects = Vec::new();
            for metric in &completion.metrics {
                effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                    metric: *metric,
                    message: format!(
                        "Os PNGs foram recuperados, mas a restauração das preferências ainda será tentada novamente: {error}"
                    ),
                }));
            }
            RecoveryCompletionAttempt {
                effects,
                retry: Some(completion),
            }
        }
    }
}

const PNG_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Default)]
struct PngRecoveryRetrySchedule {
    retry_at: Option<Duration>,
}

impl PngRecoveryRetrySchedule {
    fn request(&mut self, now: Duration, pending: bool) {
        self.retry_at = pending.then(|| now + PNG_RECOVERY_RETRY_DELAY);
    }

    fn request_if_absent(&mut self, now: Duration, pending: bool) {
        if pending && self.retry_at.is_none() {
            self.retry_at = Some(now + PNG_RECOVERY_RETRY_DELAY);
        }
    }

    fn deadline(&self) -> Option<Duration> {
        self.retry_at
    }

    fn take_due(&mut self, now: Duration) -> bool {
        if self.retry_at.is_some_and(|deadline| deadline <= now) {
            self.retry_at = None;
            true
        } else {
            false
        }
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
    metric_png_recovery: AssetTransactionRecovery<PngAssetTransaction>,
    identifier_png_recovery:
        AssetTransactionRecovery<IdentifierResetTransaction<PngAssetTransaction>>,
    indicator_reset_assets: GlobalIndicatorAssetLifecycle,
    pending_png_recovery_completions: Vec<PendingRecoveryCompletion>,
    png_recovery_schedule: PngRecoveryRetrySchedule,
    system_usage: SystemUsageSession,
    presentation: statlet::runtime_profile::RuntimePresentation,
}

struct RuntimeStores {
    preferences: PreferencesStore,
    icon_assets: IconAssetStore,
    history: HistoryStore,
}

impl RuntimeAdapters {
    fn new(
        stores: RuntimeStores,
        history: History,
        review_space_item: MenuItem,
        proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
        metrics_interval: MetricsRefreshInterval,
        presentation: statlet::runtime_profile::RuntimePresentation,
    ) -> Self {
        Self {
            preferences_store: stores.preferences,
            icon_asset_store: stores.icon_assets,
            history_store: stores.history,
            history,
            windows: None,
            samplers: RuntimeSamplers::new(metrics_interval, Some(proxy.clone())),
            mole: RuntimeMole::new(proxy.clone()),
            notifications: None,
            visual_environment_observer: None,
            visual_environment: VisualEnvironmentState::default(),
            schedule: RuntimeSchedule::new(),
            review_space_item,
            system_usage: SystemUsageSession::new(),
            event_proxy: proxy,
            png_import_generations: [0; 2],
            prepared_png_imports: [None, None],
            metric_png_recovery: AssetTransactionRecovery::default(),
            identifier_png_recovery: AssetTransactionRecovery::default(),
            indicator_reset_assets: GlobalIndicatorAssetLifecycle::default(),
            pending_png_recovery_completions: Vec::new(),
            png_recovery_schedule: PngRecoveryRetrySchedule::default(),
            presentation,
        }
    }

    fn retry_png_asset_recovery(&mut self, core: &mut StatletCore) -> Vec<AppEffect> {
        let metric = self.metric_png_recovery.retry(core);
        let identifiers = self.identifier_png_recovery.retry(core);
        let global = self.indicator_reset_assets.retry_transactions(core);
        let mut effects = metric.effects;
        effects.extend(identifiers.effects);
        effects.extend(global.effects);
        effects.extend(self.indicator_reset_assets.retry_cleanup(core));
        let mut completions = std::mem::take(&mut self.pending_png_recovery_completions);
        completions.extend(metric.completions);
        completions.extend(identifiers.completions);
        completions.extend(global.completions);
        let now = self.samplers.clock.now();
        for completion in completions {
            let attempt = persist_recovery_completion(
                &self.preferences_store,
                &mut self.schedule,
                now,
                core,
                completion,
            );
            effects.extend(attempt.effects);
            if let Some(completion) = attempt.retry {
                self.pending_png_recovery_completions.push(completion);
            }
        }
        let pending = metric.pending_transactions
            || identifiers.pending_transactions
            || global.pending_transactions
            || !self.indicator_reset_assets.retained_cleanup.is_empty()
            || !self.pending_png_recovery_completions.is_empty();
        self.png_recovery_schedule.request(now, pending);
        effects
    }

    fn has_pending_png_recovery(&self) -> bool {
        !self.metric_png_recovery.pending.is_empty()
            || !self.identifier_png_recovery.pending.is_empty()
            || !self.indicator_reset_assets.retained_transactions.is_empty()
            || !self.indicator_reset_assets.retained_cleanup.is_empty()
            || !self.pending_png_recovery_completions.is_empty()
    }

    fn ensure_png_recovery_retry_scheduled(&mut self) {
        self.png_recovery_schedule
            .request_if_absent(self.samplers.clock.now(), self.has_pending_png_recovery());
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
        self.windows = Some(WindowManager::new(
            marker,
            proxy.clone(),
            self.presentation.clone(),
            self.icon_asset_store.clone(),
        ));
        self.notifications =
            NotificationManager::new(marker, proxy.clone(), self.presentation.clone());
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
        let system_usage_deadline = self.system_usage.next_deadline();
        self.schedule
            .next_deadline(
                metrics_deadline,
                [
                    system_usage_deadline,
                    disk_deadline,
                    self.png_recovery_schedule.deadline(),
                ],
            )
            .saturating_sub(now)
    }

    fn process_due(
        &mut self,
        core: &mut StatletCore,
        renderer: &mut Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) -> bool {
        let now = self.samplers.clock.now();
        if self.png_recovery_schedule.take_due(now) {
            let recovery_effects = self.retry_png_asset_recovery(core);
            if self.apply_effects(&recovery_effects, core, renderer, button) {
                return true;
            }
        }
        let poll = self.samplers.poll_due(core);
        self.advance_system_usage_at(SystemUsageCause::Wake(poll.cycle), now);
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

    fn advance_system_usage(&mut self, cause: SystemUsageCause) {
        let now = self.samplers.clock.now();
        self.advance_system_usage_at(cause, now);
    }

    fn advance_system_usage_at(&mut self, cause: SystemUsageCause, now: Duration) {
        let mut surface = RuntimeSystemUsageSurface {
            windows: self.windows.as_ref(),
        };
        self.system_usage
            .advance(cause, now, &mut self.samplers, &mut surface);
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
                        self.advance_system_usage(SystemUsageCause::SurfaceChanged);
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
                            TransactionFailure::new(
                                "O PNG preparado não está mais disponível; a escolha anterior foi restaurada."
                                    .to_owned(),
                                Vec::new(),
                            )
                        })
                        .and_then(|prepared| {
                            self.icon_asset_store
                                .begin_replace(prepared)
                                .map_err(|error| {
                                    let (message, retained) = error.into_parts();
                                    TransactionFailure::new(message, retained)
                                })
                        }),
                        MetricPngAssetMutation::Remove => self
                            .icon_asset_store
                            .begin_remove(metric)
                            .map_err(|error| {
                                let (message, retained) = error.into_parts();
                                TransactionFailure::new(message, retained)
                            }),
                    };
                    match transaction {
                        Ok(transaction) => pending.extend(persist_metric_png_change_with_recovery(
                            &self.preferences_store,
                            &mut self.schedule,
                            self.samplers.clock.now(),
                            core,
                            metric,
                            previous,
                            preferences,
                            transaction,
                            &mut self.metric_png_recovery,
                        )),
                        Err(failure) if failure.retained.is_empty() => {
                            pending.extend(core.handle(AppEvent::MetricPngPersistenceFailed {
                                metric,
                                previous,
                                message: failure.message,
                            }))
                        }
                        Err(failure) => {
                            let message = failure.message.clone();
                            self.metric_png_recovery.retain(
                                failure,
                                TransactionRecoveryAction::Rollback,
                                Some(AppEvent::MetricPngPersistenceFailed {
                                    metric,
                                    previous,
                                    message: "A escolha de PNG anterior foi restaurada após uma nova tentativa segura."
                                        .to_owned(),
                                }),
                                vec![metric],
                            );
                            pending.extend(core.handle(AppEvent::PreferencesSaveFinished(
                                PreferencesSaveResult::Failed,
                            )));
                            pending.extend(core.handle(
                                AppEvent::MetricPngTransactionCleanupFailed { metric, message },
                            ));
                        }
                    }
                }
                AppEffect::PersistIdentifierReset {
                    previous,
                    preferences,
                } => {
                    let metrics = [MetricKind::Cpu, MetricKind::Ram]
                        .into_iter()
                        .filter(|metric| match metric {
                            MetricKind::Cpu => previous.cpu.png.is_some(),
                            MetricKind::Ram => previous.ram.png.is_some(),
                        })
                        .collect::<Vec<_>>();
                    let transaction = begin_asset_removals(&self.icon_asset_store, &metrics);
                    match transaction {
                        Ok(transaction) => pending.extend(persist_identifier_reset_with_recovery(
                            IdentifierResetPersistenceContext {
                                store: &self.preferences_store,
                                schedule: &mut self.schedule,
                                now: self.samplers.clock.now(),
                                core,
                            },
                            IdentifierResetPersistencePlan {
                                previous,
                                preferences,
                                metrics,
                                transaction: IdentifierResetTransaction::new(transaction),
                            },
                            &mut self.identifier_png_recovery,
                        )),
                        Err(failure) if failure.retained.is_empty() => pending.extend(core.handle(
                            AppEvent::IdentifierResetPersistenceFailed {
                                previous,
                                message: failure.message,
                            },
                        )),
                        Err(failure) => {
                            let message = failure.message.clone();
                            self.identifier_png_recovery.retain(
                                TransactionFailure::new(
                                    failure.message,
                                    vec![IdentifierResetTransaction::new(failure.retained)],
                                ),
                                TransactionRecoveryAction::Rollback,
                                Some(AppEvent::IdentifierResetPersistenceFailed {
                                    previous,
                                    message: "Os identificadores anteriores foram restaurados após uma nova tentativa segura."
                                        .to_owned(),
                                }),
                                metrics.clone(),
                            );
                            pending.extend(core.handle(AppEvent::PreferencesSaveFinished(
                                PreferencesSaveResult::Failed,
                            )));
                            for metric in metrics {
                                pending.extend(core.handle(
                                    AppEvent::MetricPngTransactionCleanupFailed {
                                        metric,
                                        message: message.clone(),
                                    },
                                ));
                            }
                        }
                    }
                }
                effect @ (AppEffect::PersistGlobalIndicatorReset { .. }
                | AppEffect::PersistGlobalIndicatorUndo { .. }
                | AppEffect::DiscardGlobalIndicatorUndo { .. }) => {
                    pending.extend(self.indicator_reset_assets.apply(
                        &self.icon_asset_store,
                        &self.preferences_store,
                        &mut self.schedule,
                        self.samplers.clock.now(),
                        core,
                        effect,
                    ));
                }
                AppEffect::Quit => should_quit = true,
            }
        }
        self.review_space_item
            .set_enabled(core.state().preferences.mole_integration_enabled);
        if let Some(windows) = &self.windows {
            windows.update_state(core.state());
        }
        self.ensure_png_recovery_retry_scheduled();
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
#[cfg(test)]
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
    persist_metric_png_change_with_recovery(
        store,
        schedule,
        now,
        core,
        metric,
        previous,
        preferences,
        transaction,
        &mut AssetTransactionRecovery::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_metric_png_change_with_recovery<T: MetricPngTransaction>(
    store: &impl PreferencesPersistence,
    schedule: &mut RuntimeSchedule<Preferences>,
    now: Duration,
    core: &mut StatletCore,
    metric: MetricKind,
    previous: statlet::indicator_preferences::MetricIdentifierPreferences,
    preferences: Preferences,
    transaction: T,
    recovery: &mut AssetTransactionRecovery<T>,
) -> Vec<AppEffect> {
    schedule.queue_save(now, preferences.clone());
    schedule.request_save_now(now);
    debug_assert_eq!(schedule.due_save(now), Some(preferences.clone()));
    match store.save(preferences.clone()) {
        Ok(()) => {
            let cleanup_error = transaction.commit().err().map(|failure| {
                recovery.retain(
                    failure,
                    TransactionRecoveryAction::Commit,
                    None,
                    vec![metric],
                )
            });
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
            let cleanup_error = transaction.commit().err().map(|failure| {
                recovery.retain(
                    failure,
                    TransactionRecoveryAction::Commit,
                    None,
                    vec![metric],
                )
            });
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
            match transaction.rollback() {
                Ok(()) => core.handle(AppEvent::MetricPngPersistenceFailed {
                    metric,
                    previous,
                    message: "Não foi possível salvar as preferências; a escolha de PNG anterior foi restaurada."
                        .to_owned(),
                }),
                Err(failure) => {
                    let message = format!(
                        "Não foi possível salvar as preferências nem restaurar o PNG anterior: {}",
                        failure.message
                    );
                    recovery.retain(
                        failure,
                        TransactionRecoveryAction::Rollback,
                        Some(AppEvent::MetricPngPersistenceFailed {
                            metric,
                            previous,
                            message: "A escolha de PNG anterior foi restaurada após uma nova tentativa segura."
                                .to_owned(),
                        }),
                        vec![metric],
                    );
                    let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                        PreferencesSaveResult::Failed,
                    ));
                    effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                        metric,
                        message,
                    }));
                    effects
                }
            }
        }
    }
}

trait MetricPngTransaction {
    fn commit(self) -> Result<(), TransactionFailure<Self>>
    where
        Self: Sized;
    fn rollback(self) -> Result<(), TransactionFailure<Self>>
    where
        Self: Sized;
}

#[derive(Debug)]
struct TransactionFailure<T> {
    message: String,
    retained: Vec<T>,
}

impl<T> TransactionFailure<T> {
    fn new(message: String, retained: Vec<T>) -> Self {
        Self { message, retained }
    }
}

impl<T> std::fmt::Display for TransactionFailure<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl MetricPngTransaction for PngAssetTransaction {
    fn commit(self) -> Result<(), TransactionFailure<Self>> {
        PngAssetTransaction::commit(self).map_err(|error| {
            let (message, retained) = error.into_parts();
            TransactionFailure::new(message, retained)
        })
    }

    fn rollback(self) -> Result<(), TransactionFailure<Self>> {
        PngAssetTransaction::rollback(self).map_err(|error| {
            let (message, retained) = error.into_parts();
            TransactionFailure::new(message, retained)
        })
    }
}

#[derive(Debug)]
struct IdentifierResetTransaction<T> {
    transactions: Vec<T>,
}

impl<T> IdentifierResetTransaction<T> {
    fn new(transactions: Vec<T>) -> Self {
        Self { transactions }
    }
}

impl<T: MetricPngTransaction> MetricPngTransaction for IdentifierResetTransaction<T> {
    fn commit(self) -> Result<(), TransactionFailure<Self>> {
        let mut messages = Vec::new();
        let mut retained = Vec::new();
        for transaction in self.transactions {
            if let Err(failure) = transaction.commit() {
                messages.push(failure.message);
                retained.extend(failure.retained);
            }
        }
        if messages.is_empty() {
            Ok(())
        } else {
            Err(TransactionFailure::new(
                messages.join("; "),
                vec![IdentifierResetTransaction::new(retained)],
            ))
        }
    }

    fn rollback(self) -> Result<(), TransactionFailure<Self>> {
        let mut messages = Vec::new();
        let mut retained = Vec::new();
        for transaction in self.transactions.into_iter().rev() {
            if let Err(failure) = transaction.rollback() {
                messages.push(failure.message);
                retained.extend(failure.retained);
            }
        }
        if messages.is_empty() {
            Ok(())
        } else {
            retained.reverse();
            Err(TransactionFailure::new(
                messages.join("; "),
                vec![IdentifierResetTransaction::new(retained)],
            ))
        }
    }
}

#[cfg(test)]
fn begin_identifier_reset_transaction<T, F>(
    metrics: &[MetricKind],
    mut begin_remove: F,
) -> Result<IdentifierResetTransaction<T>, String>
where
    T: MetricPngTransaction,
    F: FnMut(MetricKind) -> Result<T, String>,
{
    let mut transactions = Vec::with_capacity(metrics.len());
    for metric in metrics {
        match begin_remove(*metric) {
            Ok(transaction) => transactions.push(transaction),
            Err(error) => {
                let rollback_error = IdentifierResetTransaction::new(transactions)
                    .rollback()
                    .err()
                    .map(|failure| failure.message);
                return Err(match rollback_error {
                    Some(rollback_error) => format!(
                        "{error}; também não foi possível reverter as remoções já preparadas: {rollback_error}"
                    ),
                    None => error,
                });
            }
        }
    }
    Ok(IdentifierResetTransaction::new(transactions))
}

struct IdentifierResetPersistenceContext<'a, P> {
    store: &'a P,
    schedule: &'a mut RuntimeSchedule<Preferences>,
    now: Duration,
    core: &'a mut StatletCore,
}

struct IdentifierResetPersistencePlan<T> {
    previous: IdentifierPreferences,
    preferences: Preferences,
    metrics: Vec<MetricKind>,
    transaction: T,
}

#[cfg(test)]
fn persist_identifier_reset<P, T>(
    context: IdentifierResetPersistenceContext<'_, P>,
    plan: IdentifierResetPersistencePlan<T>,
) -> Vec<AppEffect>
where
    P: PreferencesPersistence,
    T: MetricPngTransaction,
{
    persist_identifier_reset_with_recovery(context, plan, &mut AssetTransactionRecovery::default())
}

fn persist_identifier_reset_with_recovery<P, T>(
    context: IdentifierResetPersistenceContext<'_, P>,
    plan: IdentifierResetPersistencePlan<T>,
    recovery: &mut AssetTransactionRecovery<T>,
) -> Vec<AppEffect>
where
    P: PreferencesPersistence,
    T: MetricPngTransaction,
{
    let IdentifierResetPersistenceContext {
        store,
        schedule,
        now,
        core,
    } = context;
    let IdentifierResetPersistencePlan {
        previous,
        preferences,
        metrics,
        transaction,
    } = plan;
    schedule.queue_save(now, preferences.clone());
    schedule.request_save_now(now);
    debug_assert_eq!(schedule.due_save(now), Some(preferences.clone()));
    match store.save(preferences.clone()) {
        Ok(()) => {
            let cleanup_error = transaction.commit().err().map(|failure| {
                recovery.retain(
                    failure,
                    TransactionRecoveryAction::Commit,
                    None,
                    metrics.clone(),
                )
            });
            schedule.finish_save(&preferences, true);
            let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                PreferencesSaveResult::Saved,
            ));
            if let Some(error) = cleanup_error {
                for metric in metrics {
                    effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                        metric,
                        message: format!(
                            "Os identificadores foram restaurados, mas a limpeza segura da transação do PNG falhou: {error}"
                        ),
                    }));
                }
            }
            effects
        }
        Err(error) if error.commit_state == PreferencesCommitState::Committed => {
            eprintln!("Statlet committed preferences for an identifier reset with a durability warning: {error}");
            let cleanup_error = transaction.commit().err().map(|failure| {
                recovery.retain(
                    failure,
                    TransactionRecoveryAction::Commit,
                    None,
                    metrics.clone(),
                )
            });
            schedule.finish_save(&preferences, false);
            let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                PreferencesSaveResult::Failed,
            ));
            for metric in &metrics {
                effects.extend(core.handle(AppEvent::MetricPngDurabilityWarning {
                    metric: *metric,
                    message: format!(
                        "As preferências e os PNGs foram aplicados, mas a confirmação de durabilidade falhou: {error}"
                    ),
                }));
            }
            if let Some(error) = cleanup_error {
                for metric in metrics {
                    effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                        metric,
                        message: format!(
                            "Os identificadores foram restaurados, mas a limpeza segura da transação do PNG falhou: {error}"
                        ),
                    }));
                }
            }
            effects
        }
        Err(error) => {
            eprintln!("Statlet could not save preferences for an identifier reset: {error}");
            schedule.finish_save(&preferences, false);
            match transaction.rollback() {
                Ok(()) => core.handle(AppEvent::IdentifierResetPersistenceFailed {
                    previous,
                    message: "Não foi possível salvar as preferências; os identificadores anteriores foram restaurados."
                        .to_owned(),
                }),
                Err(failure) => {
                    let message = format!(
                        "Não foi possível salvar as preferências nem restaurar os identificadores anteriores: {}",
                        failure.message
                    );
                    recovery.retain(
                        failure,
                        TransactionRecoveryAction::Rollback,
                        Some(AppEvent::IdentifierResetPersistenceFailed {
                            previous,
                            message: "Os identificadores anteriores foram restaurados após uma nova tentativa segura."
                                .to_owned(),
                        }),
                        metrics.clone(),
                    );
                    let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                        PreferencesSaveResult::Failed,
                    ));
                    for metric in metrics {
                        effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                            metric,
                            message: message.clone(),
                        }));
                    }
                    effects
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransactionRecoveryAction {
    Commit,
    Rollback,
}

struct PendingAssetTransaction<T> {
    transactions: Vec<T>,
    action: TransactionRecoveryAction,
    completion: Option<AppEvent>,
    metrics: Vec<MetricKind>,
}

struct AssetTransactionRecovery<T> {
    pending: Vec<PendingAssetTransaction<T>>,
}

impl<T> Default for AssetTransactionRecovery<T> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
        }
    }
}

struct AssetRecoveryAttempt {
    effects: Vec<AppEffect>,
    completions: Vec<PendingRecoveryCompletion>,
    pending_transactions: bool,
}

struct PendingRecoveryCompletion {
    event: AppEvent,
    metrics: Vec<MetricKind>,
}

impl<T: MetricPngTransaction> AssetTransactionRecovery<T> {
    fn retain(
        &mut self,
        failure: TransactionFailure<T>,
        action: TransactionRecoveryAction,
        completion: Option<AppEvent>,
        metrics: Vec<MetricKind>,
    ) -> String {
        let TransactionFailure { message, retained } = failure;
        self.pending.push(PendingAssetTransaction {
            transactions: retained,
            action,
            completion,
            metrics,
        });
        message
    }

    fn retry(&mut self, core: &mut StatletCore) -> AssetRecoveryAttempt {
        let mut effects = Vec::new();
        let mut retained = Vec::new();
        let mut completions = Vec::new();
        for pending in std::mem::take(&mut self.pending) {
            let PendingAssetTransaction {
                transactions,
                action,
                completion,
                metrics,
            } = pending;
            let mut failures = Vec::new();
            let mut remaining = Vec::new();
            for transaction in transactions {
                let result = match action {
                    TransactionRecoveryAction::Commit => transaction.commit(),
                    TransactionRecoveryAction::Rollback => transaction.rollback(),
                };
                if let Err(failure) = result {
                    failures.push(failure.message);
                    remaining.extend(failure.retained);
                }
            }
            if remaining.is_empty() {
                if let Some(completion) = completion {
                    completions.push(PendingRecoveryCompletion {
                        event: completion,
                        metrics,
                    });
                }
            } else {
                let message = failures.join("; ");
                for metric in &metrics {
                    effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                        metric: *metric,
                        message: format!(
                            "A recuperação do PNG ainda está pendente e será tentada novamente: {message}"
                        ),
                    }));
                }
                retained.push(PendingAssetTransaction {
                    transactions: remaining,
                    action,
                    completion,
                    metrics,
                });
            }
        }
        self.pending = retained;
        AssetRecoveryAttempt {
            effects,
            completions,
            pending_transactions: !self.pending.is_empty(),
        }
    }
}

enum GlobalRecoveryCompletion {
    Reset(GlobalIndicatorResetFailure),
    Undo(GlobalIndicatorUndoFailure),
}

struct PendingGlobalAssetRecovery {
    transactions: Vec<IdentifierResetTransaction<PngAssetTransaction>>,
    action: TransactionRecoveryAction,
    completion: Option<GlobalRecoveryCompletion>,
    metrics: Vec<MetricKind>,
}

#[derive(Default)]
struct GlobalIndicatorAssetLifecycle {
    undo: Option<IndicatorPngSnapshot>,
    retained_cleanup: Vec<IndicatorPngSnapshot>,
    retained_transactions: Vec<PendingGlobalAssetRecovery>,
}

impl GlobalIndicatorAssetLifecycle {
    fn apply(
        &mut self,
        asset_store: &IconAssetStore,
        preferences_store: &impl PreferencesPersistence,
        schedule: &mut RuntimeSchedule<Preferences>,
        now: Duration,
        core: &mut StatletCore,
        effect: AppEffect,
    ) -> Vec<AppEffect> {
        match effect {
            AppEffect::PersistGlobalIndicatorReset {
                previous,
                replaced_undo,
                preferences,
            } => self.persist_reset(
                asset_store,
                preferences_store,
                schedule,
                now,
                core,
                GlobalIndicatorResetPlan {
                    previous,
                    replaced_undo,
                    preferences,
                },
            ),
            AppEffect::PersistGlobalIndicatorUndo {
                current,
                undo,
                preferences,
            } => self.persist_undo(
                asset_store,
                preferences_store,
                schedule,
                now,
                core,
                GlobalIndicatorUndoPlan {
                    current,
                    undo,
                    preferences,
                },
            ),
            AppEffect::DiscardGlobalIndicatorUndo {
                discarded: _,
                preferences,
            } => self.discard(
                asset_store,
                preferences_store,
                schedule,
                now,
                core,
                preferences,
            ),
            _ => unreachable!("global indicator lifecycle received an unrelated effect"),
        }
    }

    fn persist_reset(
        &mut self,
        asset_store: &IconAssetStore,
        preferences_store: &impl PreferencesPersistence,
        schedule: &mut RuntimeSchedule<Preferences>,
        now: Duration,
        core: &mut StatletCore,
        plan: GlobalIndicatorResetPlan,
    ) -> Vec<AppEffect> {
        let GlobalIndicatorResetPlan {
            previous,
            replaced_undo,
            preferences,
        } = plan;
        let mut snapshot = match asset_store.capture_indicator_snapshot(&previous.identifiers) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let (message, retained_snapshot) = error.into_parts();
                if let Some(retained_snapshot) = retained_snapshot {
                    self.retained_cleanup.push(retained_snapshot);
                }
                return core.handle(AppEvent::GlobalIndicatorResetPersistenceFailed(Box::new(
                    GlobalIndicatorResetFailure {
                        previous,
                        replaced_undo,
                        message,
                    },
                )));
            }
        };
        let reset_metrics = [MetricKind::Cpu, MetricKind::Ram]
            .into_iter()
            .filter(|metric| match metric {
                MetricKind::Cpu => previous.identifiers.cpu.png.is_some(),
                MetricKind::Ram => previous.identifiers.ram.png.is_some(),
            })
            .collect::<Vec<_>>();
        let removals = match begin_asset_removals(asset_store, &reset_metrics) {
            Ok(removals) => removals,
            Err(failure) => {
                let message = failure.message.clone();
                let cleanup = snapshot.cleanup().err();
                if cleanup.is_some() {
                    self.retained_cleanup.push(snapshot);
                }
                if !failure.retained.is_empty() {
                    let message = append_warning(message, cleanup.map(|error| error.to_string()));
                    self.retain_transaction_failure(
                        TransactionFailure::new(
                            failure.message,
                            vec![IdentifierResetTransaction::new(failure.retained)],
                        ),
                        TransactionRecoveryAction::Rollback,
                        Some(GlobalRecoveryCompletion::Reset(
                            GlobalIndicatorResetFailure {
                                previous,
                                replaced_undo,
                                message: message.clone(),
                            },
                        )),
                        reset_metrics.clone(),
                    );
                    return Self::recovery_pending_warning(&message, &reset_metrics, core);
                }
                return core.handle(AppEvent::GlobalIndicatorResetPersistenceFailed(Box::new(
                    GlobalIndicatorResetFailure {
                        previous,
                        replaced_undo,
                        message: append_warning(message, cleanup.map(|error| error.to_string())),
                    },
                )));
            }
        };
        let transaction = IdentifierResetTransaction::new(removals);
        let save = save_immediately(preferences_store, schedule, now, &preferences);
        match save {
            Ok(()) => {
                let transaction_warning = transaction.commit().err().map(|failure| {
                    self.retain_transaction_failure(
                        failure,
                        TransactionRecoveryAction::Commit,
                        None,
                        reset_metrics.clone(),
                    )
                });
                let old_snapshot = self.undo.replace(snapshot);
                if let Some(old_snapshot) = old_snapshot {
                    self.cleanup_or_retain(old_snapshot, core);
                }
                schedule.finish_save(&preferences, true);
                let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                    PreferencesSaveResult::Saved,
                ));
                effects.extend(self.transaction_warning(transaction_warning, core));
                effects
            }
            Err(error) if error.commit_state == PreferencesCommitState::Committed => {
                let transaction_warning = transaction.commit().err().map(|failure| {
                    self.retain_transaction_failure(
                        failure,
                        TransactionRecoveryAction::Commit,
                        None,
                        reset_metrics.clone(),
                    )
                });
                let old_snapshot = self.undo.replace(snapshot);
                if let Some(old_snapshot) = old_snapshot {
                    self.cleanup_or_retain(old_snapshot, core);
                }
                schedule.finish_save(&preferences, false);
                let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                    PreferencesSaveResult::Failed,
                ));
                effects.extend(self.transaction_warning(
                    Some(append_warning(error.to_string(), transaction_warning)),
                    core,
                ));
                effects
            }
            Err(error) => {
                schedule.finish_save(&preferences, false);
                match transaction.rollback() {
                    Ok(()) => {
                        let cleanup = snapshot.cleanup().err().map(|error| error.to_string());
                        if cleanup.is_some() {
                            self.retained_cleanup.push(snapshot);
                        }
                        core.handle(AppEvent::GlobalIndicatorResetPersistenceFailed(Box::new(
                            GlobalIndicatorResetFailure {
                                previous,
                                replaced_undo,
                                message: append_warning(error.to_string(), cleanup),
                            },
                        )))
                    }
                    Err(failure) => {
                        let rollback_message = failure.message.clone();
                        let cleanup = snapshot.cleanup().err().map(|error| error.to_string());
                        if cleanup.is_some() {
                            self.retained_cleanup.push(snapshot);
                        }
                        let message = append_warning(
                            append_warning(error.to_string(), Some(rollback_message)),
                            cleanup,
                        );
                        self.retain_transaction_failure(
                            failure,
                            TransactionRecoveryAction::Rollback,
                            Some(GlobalRecoveryCompletion::Reset(
                                GlobalIndicatorResetFailure {
                                    previous,
                                    replaced_undo,
                                    message: message.clone(),
                                },
                            )),
                            reset_metrics.clone(),
                        );
                        Self::recovery_pending_warning(&message, &reset_metrics, core)
                    }
                }
            }
        }
    }

    fn persist_undo(
        &mut self,
        asset_store: &IconAssetStore,
        preferences_store: &impl PreferencesPersistence,
        schedule: &mut RuntimeSchedule<Preferences>,
        now: Duration,
        core: &mut StatletCore,
        plan: GlobalIndicatorUndoPlan,
    ) -> Vec<AppEffect> {
        let GlobalIndicatorUndoPlan {
            current,
            undo,
            preferences,
        } = plan;
        let undo_metrics = [MetricKind::Cpu, MetricKind::Ram]
            .into_iter()
            .filter(|metric| match metric {
                MetricKind::Cpu => {
                    current.identifiers.cpu.png.is_some() || undo.identifiers.cpu.png.is_some()
                }
                MetricKind::Ram => {
                    current.identifiers.ram.png.is_some() || undo.identifiers.ram.png.is_some()
                }
            })
            .collect::<Vec<_>>();
        let Some(snapshot) = self.undo.take() else {
            return core.handle(AppEvent::GlobalIndicatorUndoPersistenceFailed(Box::new(
                GlobalIndicatorUndoFailure {
                    current,
                    undo,
                    message: "O snapshot de assets do Undo não está disponível.".into(),
                    stage: GlobalIndicatorUndoFailureStage::AssetPreparation,
                },
            )));
        };
        let preparation = asset_store
            .begin_restore_indicator_snapshot(&snapshot, &undo.identifiers)
            .map(IdentifierResetTransaction::new)
            .map_err(|error| {
                let (message, retained) = error.into_parts();
                let retained = if retained.is_empty() {
                    Vec::new()
                } else {
                    vec![IdentifierResetTransaction::new(retained)]
                };
                TransactionFailure::new(message, retained)
            });
        self.persist_undo_after_preparation(
            GlobalIndicatorUndoPreparationContext {
                preferences_store,
                schedule,
                now,
                core,
            },
            GlobalIndicatorUndoPreparationPlan {
                undo: GlobalIndicatorUndoPlan {
                    current,
                    undo,
                    preferences,
                },
                snapshot,
                metrics: undo_metrics,
                preparation,
            },
        )
    }

    fn persist_undo_after_preparation<P: PreferencesPersistence>(
        &mut self,
        context: GlobalIndicatorUndoPreparationContext<'_, P>,
        prepared: GlobalIndicatorUndoPreparationPlan,
    ) -> Vec<AppEffect> {
        let GlobalIndicatorUndoPreparationContext {
            preferences_store,
            schedule,
            now,
            core,
        } = context;
        let GlobalIndicatorUndoPreparationPlan {
            undo: plan,
            snapshot,
            metrics: undo_metrics,
            preparation,
        } = prepared;
        let GlobalIndicatorUndoPlan {
            current,
            undo,
            preferences,
        } = plan;
        let transaction = match preparation {
            Ok(transaction) => transaction,
            Err(failure) => {
                self.undo = Some(snapshot);
                let message = failure.message.clone();
                if failure.retained.is_empty() {
                    return core.handle(AppEvent::GlobalIndicatorUndoPersistenceFailed(Box::new(
                        GlobalIndicatorUndoFailure {
                            current,
                            undo,
                            message,
                            stage: GlobalIndicatorUndoFailureStage::AssetPreparation,
                        },
                    )));
                }
                self.retain_transaction_failure(
                    failure,
                    TransactionRecoveryAction::Rollback,
                    Some(GlobalRecoveryCompletion::Undo(GlobalIndicatorUndoFailure {
                        current,
                        undo,
                        message: message.clone(),
                        stage: GlobalIndicatorUndoFailureStage::AssetPreparation,
                    })),
                    undo_metrics.clone(),
                );
                return Self::asset_preparation_recovery_pending_warning(
                    &message,
                    &undo_metrics,
                    core,
                );
            }
        };
        let save = save_immediately(preferences_store, schedule, now, &preferences);
        match save {
            Ok(()) => {
                let transaction_warning = transaction.commit().err().map(|failure| {
                    self.retain_transaction_failure(
                        failure,
                        TransactionRecoveryAction::Commit,
                        None,
                        undo_metrics.clone(),
                    )
                });
                self.cleanup_or_retain(snapshot, core);
                schedule.finish_save(&preferences, true);
                let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                    PreferencesSaveResult::Saved,
                ));
                effects.extend(self.transaction_warning(transaction_warning, core));
                effects
            }
            Err(error) if error.commit_state == PreferencesCommitState::Committed => {
                let transaction_warning = transaction.commit().err().map(|failure| {
                    self.retain_transaction_failure(
                        failure,
                        TransactionRecoveryAction::Commit,
                        None,
                        undo_metrics.clone(),
                    )
                });
                self.cleanup_or_retain(snapshot, core);
                schedule.finish_save(&preferences, false);
                let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                    PreferencesSaveResult::Failed,
                ));
                effects.extend(self.transaction_warning(
                    Some(append_warning(error.to_string(), transaction_warning)),
                    core,
                ));
                effects
            }
            Err(error) => {
                schedule.finish_save(&preferences, false);
                self.undo = Some(snapshot);
                match transaction.rollback() {
                    Ok(()) => core.handle(AppEvent::GlobalIndicatorUndoPersistenceFailed(
                        Box::new(GlobalIndicatorUndoFailure {
                            current,
                            undo,
                            message: error.to_string(),
                            stage: GlobalIndicatorUndoFailureStage::Persistence,
                        }),
                    )),
                    Err(failure) => {
                        let message =
                            append_warning(error.to_string(), Some(failure.message.clone()));
                        self.retain_transaction_failure(
                            failure,
                            TransactionRecoveryAction::Rollback,
                            Some(GlobalRecoveryCompletion::Undo(GlobalIndicatorUndoFailure {
                                current,
                                undo,
                                message: message.clone(),
                                stage: GlobalIndicatorUndoFailureStage::Persistence,
                            })),
                            undo_metrics.clone(),
                        );
                        Self::recovery_pending_warning(&message, &undo_metrics, core)
                    }
                }
            }
        }
    }

    fn discard(
        &mut self,
        asset_store: &IconAssetStore,
        preferences_store: &impl PreferencesPersistence,
        schedule: &mut RuntimeSchedule<Preferences>,
        now: Duration,
        core: &mut StatletCore,
        preferences: Preferences,
    ) -> Vec<AppEffect> {
        let Some(snapshot) = self.undo.take() else {
            return Vec::new();
        };
        let discard_metrics = [MetricKind::Cpu, MetricKind::Ram]
            .into_iter()
            .filter(|metric| match metric {
                MetricKind::Cpu => preferences.indicator.identifiers.cpu.png.is_none(),
                MetricKind::Ram => preferences.indicator.identifiers.ram.png.is_none(),
            })
            .collect::<Vec<_>>();
        let removals = match begin_unreferenced_asset_removals(
            asset_store,
            &preferences.indicator.identifiers,
        ) {
            Ok(removals) => removals,
            Err(failure) => {
                self.retained_cleanup.push(snapshot);
                let message = failure.message.clone();
                if !failure.retained.is_empty() {
                    self.retain_transaction_failure(
                        TransactionFailure::new(
                            failure.message,
                            vec![IdentifierResetTransaction::new(failure.retained)],
                        ),
                        TransactionRecoveryAction::Rollback,
                        None,
                        discard_metrics.clone(),
                    );
                    return Self::recovery_pending_warning(&message, &discard_metrics, core);
                }
                return self.transaction_warning(Some(message), core);
            }
        };
        let transaction = IdentifierResetTransaction::new(removals);
        let save = save_immediately(preferences_store, schedule, now, &preferences);
        match save {
            Ok(()) => {
                let warning = transaction.commit().err().map(|failure| {
                    self.retain_transaction_failure(
                        failure,
                        TransactionRecoveryAction::Commit,
                        None,
                        discard_metrics.clone(),
                    )
                });
                self.cleanup_or_retain(snapshot, core);
                schedule.finish_save(&preferences, true);
                let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                    PreferencesSaveResult::Saved,
                ));
                effects.extend(self.transaction_warning(warning, core));
                effects
            }
            Err(error) if error.commit_state == PreferencesCommitState::Committed => {
                let warning = transaction.commit().err().map(|failure| {
                    self.retain_transaction_failure(
                        failure,
                        TransactionRecoveryAction::Commit,
                        None,
                        discard_metrics.clone(),
                    )
                });
                self.cleanup_or_retain(snapshot, core);
                schedule.finish_save(&preferences, false);
                let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
                    PreferencesSaveResult::Failed,
                ));
                effects.extend(
                    self.transaction_warning(
                        Some(append_warning(error.to_string(), warning)),
                        core,
                    ),
                );
                effects
            }
            Err(error) => {
                schedule.finish_save(&preferences, false);
                self.retained_cleanup.push(snapshot);
                match transaction.rollback() {
                    Ok(()) => self.transaction_warning(Some(error.to_string()), core),
                    Err(failure) => {
                        let message =
                            append_warning(error.to_string(), Some(failure.message.clone()));
                        self.retain_transaction_failure(
                            failure,
                            TransactionRecoveryAction::Rollback,
                            None,
                            discard_metrics.clone(),
                        );
                        Self::recovery_pending_warning(&message, &discard_metrics, core)
                    }
                }
            }
        }
    }

    fn cleanup_or_retain(&mut self, mut snapshot: IndicatorPngSnapshot, core: &mut StatletCore) {
        if let Err(error) = snapshot.cleanup() {
            self.retained_cleanup.push(snapshot);
            let _ = self.transaction_warning(Some(error.to_string()), core);
        }
    }

    fn retain_transaction_failure(
        &mut self,
        failure: TransactionFailure<IdentifierResetTransaction<PngAssetTransaction>>,
        action: TransactionRecoveryAction,
        completion: Option<GlobalRecoveryCompletion>,
        metrics: Vec<MetricKind>,
    ) -> String {
        let TransactionFailure { message, retained } = failure;
        self.retained_transactions.push(PendingGlobalAssetRecovery {
            transactions: retained,
            action,
            completion,
            metrics,
        });
        message
    }

    fn retry_transactions(&mut self, core: &mut StatletCore) -> AssetRecoveryAttempt {
        let mut retained = Vec::new();
        let mut effects = Vec::new();
        let mut completions = Vec::new();
        let pending_transactions = std::mem::take(&mut self.retained_transactions);
        for pending in pending_transactions {
            let PendingGlobalAssetRecovery {
                transactions,
                action,
                completion,
                metrics,
            } = pending;
            let mut failures = Vec::new();
            let mut remaining = Vec::new();
            for transaction in transactions {
                let result = match action {
                    TransactionRecoveryAction::Commit => transaction.commit(),
                    TransactionRecoveryAction::Rollback => transaction.rollback(),
                };
                if let Err(failure) = result {
                    failures.push(failure.message);
                    remaining.extend(failure.retained);
                }
            }
            if remaining.is_empty() {
                match completion {
                    Some(GlobalRecoveryCompletion::Reset(failure)) => {
                        completions.push(PendingRecoveryCompletion {
                            event: AppEvent::GlobalIndicatorResetPersistenceFailed(Box::new(
                                failure,
                            )),
                            metrics,
                        })
                    }
                    Some(GlobalRecoveryCompletion::Undo(failure)) => {
                        completions.push(PendingRecoveryCompletion {
                            event: AppEvent::GlobalIndicatorUndoPersistenceFailed(Box::new(
                                failure,
                            )),
                            metrics,
                        })
                    }
                    None => {}
                }
            } else {
                let message = failures.join("; ");
                let asset_preparation = matches!(
                    &completion,
                    Some(GlobalRecoveryCompletion::Undo(failure))
                        if failure.stage == GlobalIndicatorUndoFailureStage::AssetPreparation
                );
                effects.extend(if asset_preparation {
                    Self::asset_preparation_recovery_pending_warning(&message, &metrics, core)
                } else {
                    Self::recovery_pending_warning(&message, &metrics, core)
                });
                retained.push(PendingGlobalAssetRecovery {
                    transactions: remaining,
                    action,
                    completion,
                    metrics,
                });
            }
        }
        self.retained_transactions = retained;
        AssetRecoveryAttempt {
            effects,
            completions,
            pending_transactions: !self.retained_transactions.is_empty(),
        }
    }

    fn recovery_pending_warning(
        message: &str,
        metrics: &[MetricKind],
        core: &mut StatletCore,
    ) -> Vec<AppEffect> {
        let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
            PreferencesSaveResult::Failed,
        ));
        for metric in metrics {
            effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                metric: *metric,
                message: format!(
                    "A recuperação do PNG ainda está pendente e será tentada novamente: {message}"
                ),
            }));
        }
        effects
    }

    fn asset_preparation_recovery_pending_warning(
        message: &str,
        metrics: &[MetricKind],
        core: &mut StatletCore,
    ) -> Vec<AppEffect> {
        let mut effects = Vec::new();
        for metric in metrics {
            effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                metric: *metric,
                message: format!(
                    "A preparação do Undo ainda está sendo revertida e será tentada novamente: {message}"
                ),
            }));
        }
        effects
    }

    fn retry_cleanup(&mut self, core: &mut StatletCore) -> Vec<AppEffect> {
        let mut retained = Vec::new();
        let mut effects = Vec::new();
        for mut snapshot in self.retained_cleanup.drain(..) {
            if let Err(error) = snapshot.cleanup() {
                for metric in snapshot.affected_metrics() {
                    effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
                        metric,
                        message: error.to_string(),
                    }));
                }
                retained.push(snapshot);
            }
        }
        self.retained_cleanup = retained;
        effects
    }

    fn transaction_warning(
        &self,
        warning: Option<String>,
        core: &mut StatletCore,
    ) -> Vec<AppEffect> {
        let Some(warning) = warning else {
            return Vec::new();
        };
        let message =
            format!("A alteração foi aplicada, mas a limpeza segura dos assets falhou: {warning}");
        let mut effects = core.handle(AppEvent::PreferencesSaveFinished(
            PreferencesSaveResult::Failed,
        ));
        effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
            metric: MetricKind::Cpu,
            message: message.clone(),
        }));
        effects.extend(core.handle(AppEvent::MetricPngTransactionCleanupFailed {
            metric: MetricKind::Ram,
            message,
        }));
        effects
    }
}

struct GlobalIndicatorResetPlan {
    previous: IndicatorPreferences,
    replaced_undo: Option<IndicatorPreferences>,
    preferences: Preferences,
}

struct GlobalIndicatorUndoPlan {
    current: IndicatorPreferences,
    undo: IndicatorPreferences,
    preferences: Preferences,
}

struct GlobalIndicatorUndoPreparationContext<'a, P> {
    preferences_store: &'a P,
    schedule: &'a mut RuntimeSchedule<Preferences>,
    now: Duration,
    core: &'a mut StatletCore,
}

struct GlobalIndicatorUndoPreparationPlan {
    undo: GlobalIndicatorUndoPlan,
    snapshot: IndicatorPngSnapshot,
    metrics: Vec<MetricKind>,
    preparation: Result<
        IdentifierResetTransaction<PngAssetTransaction>,
        TransactionFailure<IdentifierResetTransaction<PngAssetTransaction>>,
    >,
}

fn save_immediately(
    store: &impl PreferencesPersistence,
    schedule: &mut RuntimeSchedule<Preferences>,
    now: Duration,
    preferences: &Preferences,
) -> Result<(), PreferencesPersistenceError> {
    schedule.queue_save(now, preferences.clone());
    schedule.request_save_now(now);
    debug_assert_eq!(schedule.due_save(now), Some(preferences.clone()));
    store.save(preferences.clone())
}

fn begin_unreferenced_asset_removals(
    store: &IconAssetStore,
    identifiers: &IdentifierPreferences,
) -> Result<Vec<PngAssetTransaction>, TransactionFailure<PngAssetTransaction>> {
    let metrics = [
        (MetricKind::Cpu, identifiers.cpu.png.is_none()),
        (MetricKind::Ram, identifiers.ram.png.is_none()),
    ]
    .into_iter()
    .filter_map(|(metric, unreferenced)| unreferenced.then_some(metric))
    .collect::<Vec<_>>();
    begin_asset_removals(store, &metrics)
}

fn begin_asset_removals(
    store: &IconAssetStore,
    metrics: &[MetricKind],
) -> Result<Vec<PngAssetTransaction>, TransactionFailure<PngAssetTransaction>> {
    let mut transactions = Vec::with_capacity(metrics.len());
    for metric in metrics {
        match store.begin_remove(*metric) {
            Ok(transaction) => transactions.push(transaction),
            Err(error) => {
                let (primary, mut retained) = error.into_parts();
                let rollback = IdentifierResetTransaction::new(transactions)
                    .rollback()
                    .err();
                let message = rollback.as_ref().map_or(primary.clone(), |failure| {
                    format!(
                        "{primary}; também não foi possível reverter as remoções já preparadas: {}",
                        failure.message
                    )
                });
                if let Some(failure) = rollback {
                    for aggregate in failure.retained {
                        retained.extend(aggregate.transactions);
                    }
                }
                return Err(TransactionFailure::new(message, retained));
            }
        }
    }
    Ok(transactions)
}

fn append_warning(primary: String, warning: Option<String>) -> String {
    warning.map_or(primary.clone(), |warning| format!("{primary}; {warning}"))
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

struct RuntimeSystemUsageSurface<'a> {
    windows: Option<&'a WindowManager>,
}

impl SystemUsageSurface for RuntimeSystemUsageSurface<'_> {
    fn observe(&self) -> SurfaceObservation {
        self.windows.map_or_else(
            SurfaceObservation::default,
            WindowManager::system_usage_observation,
        )
    }

    fn apply(&mut self, presentation: SystemUsagePresentation) {
        let Some(windows) = self.windows else {
            return;
        };
        if presentation.focus_summary {
            windows.request_system_usage_summary_focus(presentation.view_model.section);
        }
        windows.update_system_usage(&presentation.view_model);
    }
}

struct RuntimeSamplers {
    metrics: MacSampler,
    metrics_schedule: MetricsSamplingSchedule,
    disk: StartupVolumeSampler,
    disk_schedule: DiskSamplingSchedule,
    clock: ContinuousClock,
    gpu: MacGpuSampler,
    process_proxy: Option<tao::event_loop::EventLoopProxy<RuntimeEvent>>,
    sampling_cycle: u64,
}

struct RuntimePoll {
    effects: Vec<AppEffect>,
    metrics_ticked: bool,
    cycle: SamplingCycle,
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
            process_proxy,
            sampling_cycle: 0,
        }
    }

    fn set_disk_sampling_enabled(&mut self, enabled: bool) {
        self.disk_schedule.set_enabled(enabled, self.clock.now());
    }

    fn poll_due(&mut self, core: &mut StatletCore) -> RuntimePoll {
        let now = self.clock.now();
        self.sampling_cycle = self.sampling_cycle.wrapping_add(1);
        let cycle = SamplingCycle::new(self.sampling_cycle);
        let metrics_ticked = self.metrics_schedule.take_due(now);
        if metrics_ticked {
            if let Some(snapshot) = self.metrics.sample_in_cycle(cycle) {
                core.handle(AppEvent::MetricsSample(snapshot.compact));
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
            cycle,
        }
    }

    fn reschedule_metrics(&mut self, interval: MetricsRefreshInterval) {
        self.metrics_schedule.reschedule(self.clock.now(), interval);
    }
}

impl SystemUsageSampling for RuntimeSamplers {
    fn memory(&mut self, cycle: SamplingCycle) -> Result<MemoryReading, ()> {
        self.metrics
            .sample_in_cycle(cycle)
            .map(|sample| sample.memory)
            .ok_or(())
    }

    fn gpu(&mut self) -> statlet::system_usage::GpuSampleOutcome {
        self.gpu.sample()
    }

    fn start_processes(&mut self, request: ProcessSampleRequest) -> ProcessStart {
        let Some(process_proxy) = self.process_proxy.as_ref() else {
            return ProcessStart::Failed;
        };
        let proxy = process_proxy.clone();
        let cancellation = request.cancellation();
        thread::Builder::new()
            .name("statlet-process-sample".to_owned())
            .spawn(move || {
                let outcome = MacSampler::sample_processes(&cancellation);
                let _ = proxy.send_event(RuntimeEvent::ProcessesSampled(request.finish(outcome)));
            })
            .map(|_| ProcessStart::Started)
            .unwrap_or(ProcessStart::Failed)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::fs;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::time::Duration;

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use statlet::core::{AppEvent, IndicatorPreferenceChange, Preferences, PreferencesSaveStatus};
    use statlet::indicator::{IndicatorRun, IndicatorScene, SegmentColor, SemanticColor};
    use statlet::indicator_preferences::{MetricIdentifierMode, MetricsRefreshInterval, SrgbColor};
    use statlet::preferences::PreferencesCommitState;
    use tempfile::tempdir;

    use super::{
        apply_persistence_intent, preview_contrast_warnings, resolved_scene_srgb_colors,
        save_preferences, spawn_png_preparation_with, visual_environment_redraw_request, AppEffect,
        GlobalIndicatorAssetLifecycle, IconAssetStore, IdentifierResetTransaction, MetricKind,
        PngAssetTransaction, PreferencesStore, PreviewContrastWarnings, RuntimeEvent,
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

    #[test]
    fn identifier_reset_rolls_back_both_assets_when_preferences_save_fails() {
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
        let cpu_metadata = asset_store
            .import_bytes(MetricKind::Cpu, "cpu.png", &png([0x11, 0x22, 0x33, 0xFF]))
            .unwrap();
        let ram_metadata = asset_store
            .import_bytes(MetricKind::Ram, "ram.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        let original_cpu = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        let original_ram = fs::read(asset_store.path_for(MetricKind::Ram)).unwrap();
        let mut core = StatletCore::new();
        for (metric, metadata) in [
            (MetricKind::Cpu, cpu_metadata),
            (MetricKind::Ram, ram_metadata),
        ] {
            core.handle(AppEvent::MetricPngImportFinished {
                metric,
                result: statlet::core::MetricPngImportResult::Imported(metadata),
            });
        }
        let previous = core.state().preferences.indicator.identifiers.clone();
        let (reset_previous, preferences) = core
            .handle(AppEvent::ResetIndicatorGroup(
                statlet::indicator_preferences::IndicatorPreferenceGroup::Identifiers,
            ))
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistIdentifierReset {
                    previous,
                    preferences,
                } => Some((previous, preferences)),
                _ => None,
            })
            .unwrap();
        let transaction = IdentifierResetTransaction::new(vec![
            asset_store.begin_remove(MetricKind::Cpu).unwrap(),
            asset_store.begin_remove(MetricKind::Ram).unwrap(),
        ]);
        let blocker = directory.path().join("not-a-directory");
        fs::write(&blocker, b"block preference parent").unwrap();
        let preferences_store = PreferencesStore::new(blocker.join("preferences.json"));
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_identifier_reset(
            super::IdentifierResetPersistenceContext {
                store: &preferences_store,
                schedule: &mut schedule,
                now: Duration::ZERO,
                core: &mut core,
            },
            super::IdentifierResetPersistencePlan {
                previous: reset_previous,
                preferences,
                metrics: vec![MetricKind::Cpu, MetricKind::Ram],
                transaction,
            },
        );

        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            original_cpu
        );
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Ram)).unwrap(),
            original_ram
        );
        assert_eq!(core.state().preferences.indicator.identifiers, previous);
        assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
    }

    #[derive(Debug)]
    struct FaultInjectedTransaction {
        commit_error: Option<String>,
        rollback_error: Option<String>,
    }

    #[derive(Debug)]
    struct RecordedTransaction {
        commit_attempted: Rc<Cell<bool>>,
        rollback_attempted: Rc<Cell<bool>>,
        commit_error: Option<String>,
        rollback_error: Option<String>,
    }

    #[derive(Debug)]
    struct RetryOnceTransaction {
        rollback_attempts: Rc<Cell<u8>>,
    }

    impl super::MetricPngTransaction for RetryOnceTransaction {
        fn commit(self) -> Result<(), super::TransactionFailure<Self>> {
            Ok(())
        }

        fn rollback(self) -> Result<(), super::TransactionFailure<Self>> {
            let attempts = self.rollback_attempts.get();
            self.rollback_attempts.set(attempts + 1);
            if attempts == 0 {
                Err(super::TransactionFailure::new(
                    "rollback fault injected once".into(),
                    vec![self],
                ))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn grouped_recovery_waits_for_every_owner_and_reports_the_ram_metric() {
        let mut core = StatletCore::new();
        let previous = core.state().preferences.indicator.identifiers.ram.clone();
        let mut recovery = super::AssetTransactionRecovery::default();
        recovery.retain(
            super::TransactionFailure::new(
                "two rollback owners retained".into(),
                vec![
                    RetryOnceTransaction {
                        rollback_attempts: Rc::new(Cell::new(1)),
                    },
                    RetryOnceTransaction {
                        rollback_attempts: Rc::new(Cell::new(0)),
                    },
                ],
            ),
            super::TransactionRecoveryAction::Rollback,
            Some(AppEvent::MetricPngPersistenceFailed {
                metric: MetricKind::Ram,
                previous,
                message: "RAM restaurada".into(),
            }),
            vec![MetricKind::Ram],
        );

        let first = recovery.retry(&mut core);

        assert!(first.completions.is_empty());
        assert!(first.pending_transactions);
        assert_eq!(recovery.pending.len(), 1);
        assert!(core.state().indicator_icon_error(MetricKind::Cpu).is_none());
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Ram)
            .unwrap()
            .contains("rollback fault injected once"));

        let second = recovery.retry(&mut core);

        assert_eq!(second.completions.len(), 1);
        assert!(!second.pending_transactions);
        assert!(recovery.pending.is_empty());
    }

    #[test]
    fn timed_png_recovery_progresses_without_a_second_user_event() {
        let mut core = StatletCore::new();
        let mut recovery = super::AssetTransactionRecovery::default();
        recovery.retain(
            super::TransactionFailure::new(
                "rollback owner retained".into(),
                vec![RetryOnceTransaction {
                    rollback_attempts: Rc::new(Cell::new(1)),
                }],
            ),
            super::TransactionRecoveryAction::Rollback,
            None,
            vec![MetricKind::Cpu],
        );
        let mut schedule = super::PngRecoveryRetrySchedule::default();

        schedule.request_if_absent(Duration::ZERO, !recovery.pending.is_empty());

        assert_eq!(schedule.deadline(), Some(Duration::from_millis(250)));
        assert!(!schedule.take_due(Duration::from_millis(249)));
        assert_eq!(recovery.pending.len(), 1);
        assert!(schedule.take_due(Duration::from_millis(250)));
        let attempt = recovery.retry(&mut core);
        schedule.request(Duration::from_millis(250), attempt.pending_transactions);
        assert!(recovery.pending.is_empty());
        assert_eq!(schedule.deadline(), None);
    }

    #[test]
    fn recovered_asset_document_is_really_saved_before_core_metadata_is_restored() {
        let directory = tempdir().unwrap();
        let store = PreferencesStore::new(directory.path().join("preferences.json"));
        let mut core = StatletCore::new();
        let mut previous = core.state().preferences.indicator.identifiers.cpu.clone();
        previous.mode = MetricIdentifierMode::Png;
        previous.png = Some(
            statlet::indicator_preferences::PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap(),
        );
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
        let completion = super::PendingRecoveryCompletion {
            event: AppEvent::MetricPngPersistenceFailed {
                metric: MetricKind::Cpu,
                previous: previous.clone(),
                message: "CPU restaurada".into(),
            },
            metrics: vec![MetricKind::Cpu],
        };

        let attempt = super::persist_recovery_completion(
            &store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            completion,
        );

        assert!(attempt.retry.is_none());
        assert_eq!(store.load().indicator.identifiers.cpu, previous);
        assert_eq!(core.state().preferences.indicator.identifiers.cpu, previous);
        assert_eq!(schedule.pending_save(), None);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
    }

    #[test]
    fn failed_recovery_save_keeps_core_metadata_current_and_retains_the_completion() {
        let directory = tempdir().unwrap();
        let blocker = directory.path().join("not-a-directory");
        fs::write(&blocker, b"blocking file").unwrap();
        let store = PreferencesStore::new(blocker.join("preferences.json"));
        let mut core = StatletCore::new();
        let current = core.state().preferences.indicator.identifiers.ram.clone();
        let mut previous = current.clone();
        previous.mode = MetricIdentifierMode::Png;
        previous.png = Some(
            statlet::indicator_preferences::PngIconMetadata::new("ram.png", 12, 12, 400).unwrap(),
        );
        let mut expected = core.state().preferences.clone();
        expected.indicator.identifiers.ram = previous.clone();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
        let completion = super::PendingRecoveryCompletion {
            event: AppEvent::MetricPngPersistenceFailed {
                metric: MetricKind::Ram,
                previous,
                message: "RAM restaurada".into(),
            },
            metrics: vec![MetricKind::Ram],
        };

        let attempt = super::persist_recovery_completion(
            &store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            completion,
        );

        assert!(attempt.retry.is_some());
        assert_eq!(core.state().preferences.indicator.identifiers.ram, current);
        assert_eq!(schedule.pending_save(), Some(&expected));
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Ram)
            .unwrap()
            .contains("restauração das preferências"));
    }

    impl super::MetricPngTransaction for RecordedTransaction {
        fn commit(self) -> Result<(), super::TransactionFailure<Self>> {
            self.commit_attempted.set(true);
            match self.commit_error.clone() {
                Some(error) => Err(super::TransactionFailure::new(error, vec![self])),
                None => Ok(()),
            }
        }

        fn rollback(self) -> Result<(), super::TransactionFailure<Self>> {
            self.rollback_attempted.set(true);
            match self.rollback_error.clone() {
                Some(error) => Err(super::TransactionFailure::new(error, vec![self])),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn identifier_reset_commit_attempts_every_transaction_and_aggregates_warnings() {
        let first_attempted = Rc::new(Cell::new(false));
        let second_attempted = Rc::new(Cell::new(false));
        let transaction = IdentifierResetTransaction::new(vec![
            RecordedTransaction {
                commit_attempted: Rc::clone(&first_attempted),
                rollback_attempted: Rc::new(Cell::new(false)),
                commit_error: Some("CPU cleanup failed".into()),
                rollback_error: None,
            },
            RecordedTransaction {
                commit_attempted: Rc::clone(&second_attempted),
                rollback_attempted: Rc::new(Cell::new(false)),
                commit_error: Some("RAM cleanup failed".into()),
                rollback_error: None,
            },
        ]);

        let error = super::MetricPngTransaction::commit(transaction).unwrap_err();

        assert!(first_attempted.get());
        assert!(second_attempted.get());
        assert_eq!(error.message, "CPU cleanup failed; RAM cleanup failed");
        assert_eq!(error.retained.len(), 1);
        assert_eq!(error.retained[0].transactions.len(), 2);
    }

    #[test]
    fn identifier_reset_rollback_attempts_every_transaction_and_aggregates_warnings() {
        let cpu_attempted = Rc::new(Cell::new(false));
        let ram_attempted = Rc::new(Cell::new(false));
        let transaction = IdentifierResetTransaction::new(vec![
            RecordedTransaction {
                commit_attempted: Rc::new(Cell::new(false)),
                rollback_attempted: Rc::clone(&cpu_attempted),
                commit_error: None,
                rollback_error: Some("CPU rollback failed".into()),
            },
            RecordedTransaction {
                commit_attempted: Rc::new(Cell::new(false)),
                rollback_attempted: Rc::clone(&ram_attempted),
                commit_error: None,
                rollback_error: Some("RAM rollback failed".into()),
            },
        ]);

        let error = super::MetricPngTransaction::rollback(transaction).unwrap_err();

        assert!(ram_attempted.get());
        assert!(cpu_attempted.get());
        assert_eq!(error.message, "RAM rollback failed; CPU rollback failed");
        assert_eq!(error.retained.len(), 1);
        assert_eq!(error.retained[0].transactions.len(), 2);
    }

    #[test]
    fn identifier_reset_retains_one_and_two_failed_rollbacks_before_restoring_metadata() {
        for metric_count in [1_usize, 2] {
            let directory = tempdir().unwrap();
            let blocked_parent = directory.path().join("not-a-directory");
            fs::write(&blocked_parent, b"blocking file").unwrap();
            let store = PreferencesStore::new(blocked_parent.join("preferences.json"));
            let mut core = StatletCore::new();
            core.handle(AppEvent::MetricPngImportFinished {
                metric: MetricKind::Cpu,
                result: statlet::core::MetricPngImportResult::Imported(
                    statlet::indicator_preferences::PngIconMetadata::new("cpu.png", 12, 12, 400)
                        .unwrap(),
                ),
            });
            if metric_count == 2 {
                core.handle(AppEvent::MetricPngImportFinished {
                    metric: MetricKind::Ram,
                    result: statlet::core::MetricPngImportResult::Imported(
                        statlet::indicator_preferences::PngIconMetadata::new(
                            "ram.png", 12, 12, 400,
                        )
                        .unwrap(),
                    ),
                });
            }
            let previous = core.state().preferences.indicator.identifiers.clone();
            let reset = core.handle(AppEvent::ResetIndicatorGroup(
                statlet::indicator_preferences::IndicatorPreferenceGroup::Identifiers,
            ));
            let preferences = reset
                .into_iter()
                .find_map(|effect| match effect {
                    AppEffect::PersistIdentifierReset { preferences, .. } => Some(preferences),
                    _ => None,
                })
                .unwrap();
            let current = core.state().preferences.indicator.identifiers.clone();
            let transactions = (0..metric_count)
                .map(|_| RetryOnceTransaction {
                    rollback_attempts: Rc::new(Cell::new(0)),
                })
                .collect();
            let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
            let mut recovery = super::AssetTransactionRecovery::default();

            super::persist_identifier_reset_with_recovery(
                super::IdentifierResetPersistenceContext {
                    store: &store,
                    schedule: &mut schedule,
                    now: Duration::ZERO,
                    core: &mut core,
                },
                super::IdentifierResetPersistencePlan {
                    previous: previous.clone(),
                    preferences,
                    metrics: [MetricKind::Cpu, MetricKind::Ram]
                        .into_iter()
                        .take(metric_count)
                        .collect(),
                    transaction: IdentifierResetTransaction::new(transactions),
                },
                &mut recovery,
            );

            assert_eq!(core.state().preferences.indicator.identifiers, current);
            assert_eq!(recovery.pending.len(), 1);
            assert_eq!(
                core.state().preferences_save_status,
                PreferencesSaveStatus::Failed
            );

            let retried = recovery.retry(&mut core);

            assert_eq!(retried.completions.len(), 1);
            assert!(!retried.pending_transactions);
            assert!(recovery.pending.is_empty());
            assert_eq!(core.state().preferences.indicator.identifiers, current);
            let completion = retried.completions.into_iter().next().unwrap();
            core.handle(completion.event);
            assert_eq!(core.state().preferences.indicator.identifiers, previous);
        }
    }

    #[test]
    fn second_identifier_removal_failure_restores_the_first_asset_and_preferences() {
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
        let cpu_metadata = asset_store
            .import_bytes(MetricKind::Cpu, "cpu.png", &png([0x11, 0x22, 0x33, 0xFF]))
            .unwrap();
        let ram_metadata = asset_store
            .import_bytes(MetricKind::Ram, "ram.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        let original_cpu = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        let original_ram = fs::read(asset_store.path_for(MetricKind::Ram)).unwrap();
        let mut core = StatletCore::new();
        for (metric, metadata) in [
            (MetricKind::Cpu, cpu_metadata),
            (MetricKind::Ram, ram_metadata),
        ] {
            core.handle(AppEvent::MetricPngImportFinished {
                metric,
                result: statlet::core::MetricPngImportResult::Imported(metadata),
            });
        }
        let previous = core.state().preferences.clone();
        core.handle(AppEvent::ResetIndicatorGroup(
            statlet::indicator_preferences::IndicatorPreferenceGroup::Identifiers,
        ));

        let result: Result<IdentifierResetTransaction<PngAssetTransaction>, String> =
            super::begin_identifier_reset_transaction(
                &[MetricKind::Cpu, MetricKind::Ram],
                |metric| match metric {
                    MetricKind::Cpu => asset_store
                        .begin_remove(metric)
                        .map_err(|error| error.user_message().to_owned()),
                    MetricKind::Ram => Err("fault injected while preparing RAM removal".into()),
                },
            );
        let message = result.unwrap_err();
        let effects = core.handle(AppEvent::IdentifierResetPersistenceFailed {
            previous: previous.indicator.identifiers.clone(),
            message: message.clone(),
        });

        assert!(message.contains("fault injected while preparing RAM removal"));
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            original_cpu
        );
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Ram)).unwrap(),
            original_ram
        );
        assert_eq!(core.state().preferences, previous);
        assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
    }

    #[test]
    fn identifier_removal_setup_error_includes_compensating_rollback_warning() {
        let rollback_attempted = Rc::new(Cell::new(false));
        let result = super::begin_identifier_reset_transaction(
            &[MetricKind::Cpu, MetricKind::Ram],
            |metric| match metric {
                MetricKind::Cpu => Ok(RecordedTransaction {
                    commit_attempted: Rc::new(Cell::new(false)),
                    rollback_attempted: Rc::clone(&rollback_attempted),
                    commit_error: None,
                    rollback_error: Some("CPU rollback failed".into()),
                }),
                MetricKind::Ram => Err("RAM removal setup failed".into()),
            },
        );

        let error = result.unwrap_err();

        assert!(rollback_attempted.get());
        assert!(error.contains("RAM removal setup failed"));
        assert!(error.contains("CPU rollback failed"));
    }

    #[test]
    fn identifier_reset_without_png_persists_without_asset_transactions() {
        let directory = tempdir().unwrap();
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let mut core = StatletCore::new();
        core.handle(AppEvent::UpdateIndicator(
            statlet::core::IndicatorPreferenceChange::SetMetricSystemSymbol {
                metric: MetricKind::Cpu,
                symbol: statlet::indicator_preferences::SystemSymbolName::new("waveform.path.ecg")
                    .unwrap(),
            },
        ));
        let (previous, preferences) = core
            .handle(AppEvent::ResetIndicatorGroup(
                statlet::indicator_preferences::IndicatorPreferenceGroup::Identifiers,
            ))
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistIdentifierReset {
                    previous,
                    preferences,
                } => Some((previous, preferences)),
                _ => None,
            })
            .unwrap();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_identifier_reset(
            super::IdentifierResetPersistenceContext {
                store: &preferences_store,
                schedule: &mut schedule,
                now: Duration::ZERO,
                core: &mut core,
            },
            super::IdentifierResetPersistencePlan {
                previous,
                preferences: preferences.clone(),
                metrics: Vec::new(),
                transaction: IdentifierResetTransaction::<PngAssetTransaction>::new(Vec::new()),
            },
        );

        assert!(effects.is_empty());
        assert_eq!(preferences_store.load(), preferences);
        assert_eq!(core.state().preferences, preferences);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
        assert_eq!(schedule.pending_save(), None);
    }

    #[test]
    fn identifier_reset_with_one_png_commits_asset_and_preferences_together() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let image = RgbaImage::from_pixel(12, 12, Rgba([0x11, 0x22, 0x33, 0xFF]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let metadata = asset_store
            .import_bytes(MetricKind::Cpu, "cpu.png", &bytes.into_inner())
            .unwrap();
        let mut core = StatletCore::new();
        core.handle(AppEvent::MetricPngImportFinished {
            metric: MetricKind::Cpu,
            result: statlet::core::MetricPngImportResult::Imported(metadata),
        });
        let (previous, preferences) = core
            .handle(AppEvent::ResetIndicatorGroup(
                statlet::indicator_preferences::IndicatorPreferenceGroup::Identifiers,
            ))
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistIdentifierReset {
                    previous,
                    preferences,
                } => Some((previous, preferences)),
                _ => None,
            })
            .unwrap();
        let metrics = vec![MetricKind::Cpu];
        let transaction = super::begin_identifier_reset_transaction(&metrics, |metric| {
            asset_store
                .begin_remove(metric)
                .map_err(|error| error.user_message().to_owned())
        })
        .unwrap();
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_identifier_reset(
            super::IdentifierResetPersistenceContext {
                store: &preferences_store,
                schedule: &mut schedule,
                now: Duration::ZERO,
                core: &mut core,
            },
            super::IdentifierResetPersistencePlan {
                previous,
                preferences: preferences.clone(),
                metrics,
                transaction,
            },
        );

        assert!(effects.is_empty());
        assert!(!asset_store.path_for(MetricKind::Cpu).exists());
        assert_eq!(preferences_store.load(), preferences);
        assert_eq!(core.state().preferences, preferences);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
    }

    #[test]
    fn identifier_reset_cleanup_failure_warns_every_affected_metric_after_save() {
        let directory = tempdir().unwrap();
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let mut core = StatletCore::new();
        let preferences = core.state().preferences.clone();
        let previous = preferences.indicator.identifiers.clone();
        let cpu_attempted = Rc::new(Cell::new(false));
        let ram_attempted = Rc::new(Cell::new(false));
        let transaction = IdentifierResetTransaction::new(vec![
            RecordedTransaction {
                commit_attempted: Rc::clone(&cpu_attempted),
                rollback_attempted: Rc::new(Cell::new(false)),
                commit_error: Some("CPU cleanup failed".into()),
                rollback_error: None,
            },
            RecordedTransaction {
                commit_attempted: Rc::clone(&ram_attempted),
                rollback_attempted: Rc::new(Cell::new(false)),
                commit_error: Some("RAM cleanup failed".into()),
                rollback_error: None,
            },
        ]);
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_identifier_reset(
            super::IdentifierResetPersistenceContext {
                store: &preferences_store,
                schedule: &mut schedule,
                now: Duration::ZERO,
                core: &mut core,
            },
            super::IdentifierResetPersistencePlan {
                previous,
                preferences: preferences.clone(),
                metrics: vec![MetricKind::Cpu, MetricKind::Ram],
                transaction,
            },
        );

        assert!(effects.is_empty());
        assert!(cpu_attempted.get());
        assert!(ram_attempted.get());
        assert_eq!(preferences_store.load(), preferences);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Saved
        );
        for metric in [MetricKind::Cpu, MetricKind::Ram] {
            let warning = core.state().indicator_icon_error(metric).unwrap();
            assert!(warning.contains("CPU cleanup failed"));
            assert!(warning.contains("RAM cleanup failed"));
        }
    }

    #[test]
    fn identifier_reset_post_rename_failure_keeps_json_assets_and_runtime_aligned() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let image = RgbaImage::from_pixel(12, 12, Rgba([0x11, 0x22, 0x33, 0xFF]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let metadata = asset_store
            .import_bytes(MetricKind::Cpu, "cpu.png", &bytes.into_inner())
            .unwrap();
        let mut core = StatletCore::new();
        core.handle(AppEvent::MetricPngImportFinished {
            metric: MetricKind::Cpu,
            result: statlet::core::MetricPngImportResult::Imported(metadata),
        });
        let (previous, preferences) = core
            .handle(AppEvent::ResetIndicatorGroup(
                statlet::indicator_preferences::IndicatorPreferenceGroup::Identifiers,
            ))
            .into_iter()
            .find_map(|effect| match effect {
                AppEffect::PersistIdentifierReset {
                    previous,
                    preferences,
                } => Some((previous, preferences)),
                _ => None,
            })
            .unwrap();
        let metrics = vec![MetricKind::Cpu];
        let transaction = super::begin_identifier_reset_transaction(&metrics, |metric| {
            asset_store
                .begin_remove(metric)
                .map_err(|error| error.user_message().to_owned())
        })
        .unwrap();
        let fault_store = PostRenameFaultStore {
            inner: PreferencesStore::new(directory.path().join("preferences.json")),
        };
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let effects = super::persist_identifier_reset(
            super::IdentifierResetPersistenceContext {
                store: &fault_store,
                schedule: &mut schedule,
                now: Duration::ZERO,
                core: &mut core,
            },
            super::IdentifierResetPersistencePlan {
                previous,
                preferences: preferences.clone(),
                metrics,
                transaction,
            },
        );

        assert!(effects.is_empty());
        assert!(!asset_store.path_for(MetricKind::Cpu).exists());
        assert_eq!(fault_store.inner.load(), preferences);
        assert_eq!(core.state().preferences, preferences);
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

    struct PostRenameFaultStore {
        inner: PreferencesStore,
    }

    struct PostRenameOnceStore {
        inner: PreferencesStore,
        fail_after_next_save: Cell<bool>,
    }

    struct SnapshotCleanupBlockingStore {
        icons: std::path::PathBuf,
        blocked_snapshot: RefCell<Option<std::path::PathBuf>>,
    }

    impl super::PreferencesPersistence for SnapshotCleanupBlockingStore {
        fn save(
            &self,
            _preferences: Preferences,
        ) -> Result<(), super::PreferencesPersistenceError> {
            let snapshot = fs::read_dir(&self.icons)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.is_dir()
                        && (path.join("cpu.png").exists() || path.join("ram.png").exists())
                })
                .expect("snapshot directory exists before save");
            fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o500)).unwrap();
            self.blocked_snapshot.replace(Some(snapshot));
            Err(super::PreferencesPersistenceError {
                commit_state: PreferencesCommitState::NotCommitted,
                message: "fault injected before preferences commit".into(),
            })
        }
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
        fn commit(self) -> Result<(), super::TransactionFailure<Self>> {
            match self.commit_error.clone() {
                Some(error) => Err(super::TransactionFailure::new(error, vec![self])),
                None => Ok(()),
            }
        }

        fn rollback(self) -> Result<(), super::TransactionFailure<Self>> {
            match self.rollback_error.clone() {
                Some(error) => Err(super::TransactionFailure::new(error, vec![self])),
                None => Ok(()),
            }
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
        let mut recovery = super::AssetTransactionRecovery::default();

        let effects = super::persist_metric_png_change_with_recovery(
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
            &mut recovery,
        );

        assert!(effects.is_empty());
        assert_eq!(recovery.pending.len(), 1);
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
        let mut recovery = super::AssetTransactionRecovery::default();

        let effects = super::persist_metric_png_change_with_recovery(
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
            &mut recovery,
        );

        assert!(effects.is_empty());
        assert_eq!(recovery.pending.len(), 1);
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
                trailing_spacing_level: 0,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".into(),
                color: gray,
                trailing_spacing_level: 0,
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
                trailing_spacing_level: 0,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".into(),
                color: warning,
                trailing_spacing_level: 0,
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

    #[test]
    fn global_reset_then_png_replacement_and_undo_restore_original_bytes_and_document() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let original_source = png_bytes([0x11, 0x22, 0x33, 0xFF]);
        let replacement_source = png_bytes([0xAA, 0xBB, 0xCC, 0xFF]);
        let original_metadata = asset_store
            .import_bytes(MetricKind::Cpu, "original.png", &original_source)
            .unwrap();
        let original_bytes = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        let mut core = StatletCore::new();
        core.handle(AppEvent::MetricPngImportFinished {
            metric: MetricKind::Cpu,
            result: statlet::core::MetricPngImportResult::Imported(original_metadata.clone()),
        });
        let original_preferences = core.state().preferences.clone();
        preferences_store
            .save(original_preferences.clone())
            .unwrap();
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let reset_effect = core
            .handle(AppEvent::ResetIndicatorConfirmed)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            reset_effect,
        );
        assert!(!asset_store.path_for(MetricKind::Cpu).exists());

        let prepared = asset_store
            .prepare_bytes(MetricKind::Cpu, "replacement.png", &replacement_source)
            .unwrap();
        let replacement_metadata = prepared.metadata().clone();
        let transaction = asset_store.begin_replace(prepared).unwrap();
        let replacement_effect = core
            .handle(AppEvent::MetricPngImportFinished {
                metric: MetricKind::Cpu,
                result: statlet::core::MetricPngImportResult::Imported(replacement_metadata),
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
        super::persist_metric_png_change(
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            replacement_effect.0,
            replacement_effect.1,
            replacement_effect.2,
            transaction,
        );
        assert_ne!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            original_bytes
        );

        let undo_effect = core
            .handle(AppEvent::UndoIndicatorReset)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorUndo { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            undo_effect,
        );

        let restored_bytes = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        assert_eq!(restored_bytes, original_bytes);
        assert_eq!(
            statlet::indicator_preferences::PngIconMetadata::with_content_fingerprint(
                original_metadata.source_name(),
                original_metadata.width(),
                original_metadata.height(),
                restored_bytes.len() as u64,
                test_content_fingerprint(&restored_bytes),
            )
            .unwrap(),
            original_metadata
        );
        assert_eq!(core.state().preferences, original_preferences);
        assert_eq!(preferences_store.load(), original_preferences);
        assert!(!core.state().can_undo_indicator_reset);
    }

    #[test]
    fn closing_after_global_reset_removes_retained_and_unreferenced_png_assets() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let metadata = asset_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png_bytes([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let mut core = StatletCore::new();
        core.handle(AppEvent::MetricPngImportFinished {
            metric: MetricKind::Cpu,
            result: statlet::core::MetricPngImportResult::Imported(metadata),
        });
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
        let reset = core
            .handle(AppEvent::ResetIndicatorConfirmed)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            reset,
        );

        let discard = core
            .handle(AppEvent::PreferencesWindowClosed)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::DiscardGlobalIndicatorUndo { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            discard,
        );

        assert!(!asset_store.path_for(MetricKind::Cpu).exists());
        assert_eq!(
            fs::read_dir(directory.path().join("icons"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(preferences_store.load(), core.state().preferences);
    }

    #[test]
    fn global_reset_with_one_png_removes_only_its_canonical_and_undo_restores_exact_bytes() {
        for metric in [MetricKind::Cpu, MetricKind::Ram] {
            let other = match metric {
                MetricKind::Cpu => MetricKind::Ram,
                MetricKind::Ram => MetricKind::Cpu,
            };
            let directory = tempdir().unwrap();
            let icons = directory.path().join("icons");
            let asset_store = IconAssetStore::new(icons.clone());
            let preferences_store =
                PreferencesStore::new(directory.path().join("preferences.json"));
            let original = png_bytes(match metric {
                MetricKind::Cpu => [0x11, 0x22, 0x33, 0xFF],
                MetricKind::Ram => [0x44, 0x55, 0x66, 0xFF],
            });
            let metadata = asset_store
                .import_bytes(metric, "original.png", &original)
                .unwrap();
            let mut core = StatletCore::new();
            core.handle(AppEvent::MetricPngImportFinished {
                metric,
                result: statlet::core::MetricPngImportResult::Imported(metadata),
            });
            let previous = core.state().preferences.clone();
            preferences_store.save(previous.clone()).unwrap();
            let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
            let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

            let reset = core
                .handle(AppEvent::ResetIndicatorConfirmed)
                .into_iter()
                .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
                .unwrap();
            lifecycle.apply(
                &asset_store,
                &preferences_store,
                &mut schedule,
                Duration::ZERO,
                &mut core,
                reset,
            );

            assert!(!asset_store.path_for(metric).exists());
            assert!(!asset_store.path_for(other).exists());
            assert_eq!(fs::read_dir(&icons).unwrap().count(), 1);

            let undo = core
                .handle(AppEvent::UndoIndicatorReset)
                .into_iter()
                .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorUndo { .. }))
                .unwrap();
            lifecycle.apply(
                &asset_store,
                &preferences_store,
                &mut schedule,
                Duration::ZERO,
                &mut core,
                undo,
            );

            assert_eq!(fs::read(asset_store.path_for(metric)).unwrap(), original);
            assert!(!asset_store.path_for(other).exists());
            assert_eq!(core.state().preferences, previous);
            assert_eq!(preferences_store.load(), previous);
        }
    }

    #[test]
    fn global_reset_with_two_pngs_keeps_only_the_owned_snapshot_until_undo() {
        let directory = tempdir().unwrap();
        let icons = directory.path().join("icons");
        let asset_store = IconAssetStore::new(icons.clone());
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let cpu_bytes = png_bytes([0x11, 0x22, 0x33, 0xFF]);
        let ram_bytes = png_bytes([0x44, 0x55, 0x66, 0xFF]);
        let cpu = asset_store
            .import_bytes(MetricKind::Cpu, "cpu.png", &cpu_bytes)
            .unwrap();
        let ram = asset_store
            .import_bytes(MetricKind::Ram, "ram.png", &ram_bytes)
            .unwrap();
        let mut preferences = Preferences::default();
        preferences.indicator.identifiers.cpu.mode = MetricIdentifierMode::Png;
        preferences.indicator.identifiers.cpu.png = Some(cpu);
        preferences.indicator.identifiers.ram.mode = MetricIdentifierMode::Png;
        preferences.indicator.identifiers.ram.png = Some(ram);
        let (mut core, _) = StatletCore::with_preferences(preferences.clone());
        preferences_store.save(preferences.clone()).unwrap();
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        let reset = core
            .handle(AppEvent::ResetIndicatorConfirmed)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            reset,
        );

        assert!(!asset_store.path_for(MetricKind::Cpu).exists());
        assert!(!asset_store.path_for(MetricKind::Ram).exists());
        assert_eq!(fs::read_dir(&icons).unwrap().count(), 1);

        let undo = core
            .handle(AppEvent::UndoIndicatorReset)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorUndo { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            undo,
        );

        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            cpu_bytes
        );
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Ram)).unwrap(),
            ram_bytes
        );
        assert_eq!(core.state().preferences, preferences);
        assert_eq!(preferences_store.load(), preferences);
    }

    #[test]
    fn second_global_reset_without_pngs_performs_no_asset_io() {
        let directory = tempdir().unwrap();
        let icons = directory.path().join("icons");
        let asset_store = IconAssetStore::new(icons.clone());
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let mut preferences = Preferences::default();
        preferences.indicator.typography.size =
            statlet::indicator_preferences::FontSize::try_from(14).unwrap();
        let (mut core, _) = StatletCore::with_preferences(preferences);
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        for _ in 0..2 {
            let reset = core
                .handle(AppEvent::ResetIndicatorConfirmed)
                .into_iter()
                .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
                .unwrap();
            lifecycle.apply(
                &asset_store,
                &preferences_store,
                &mut schedule,
                Duration::ZERO,
                &mut core,
                reset,
            );
        }

        assert!(!icons.exists());
        assert!(core.state().can_undo_indicator_reset);
    }

    #[test]
    fn failed_global_reset_retains_snapshot_when_abort_cleanup_needs_retry() {
        let directory = tempdir().unwrap();
        let icons = directory.path().join("icons");
        let asset_store = IconAssetStore::new(icons.clone());
        let metadata = asset_store
            .import_bytes(
                MetricKind::Ram,
                "ram.png",
                &png_bytes([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let original_bytes = fs::read(asset_store.path_for(MetricKind::Ram)).unwrap();
        let mut preferences = Preferences::default();
        preferences.indicator.identifiers.ram.mode = MetricIdentifierMode::Png;
        preferences.indicator.identifiers.ram.png = Some(metadata);
        let (mut core, _) = StatletCore::with_preferences(preferences.clone());
        let failing_store = SnapshotCleanupBlockingStore {
            icons,
            blocked_snapshot: RefCell::new(None),
        };
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
        let reset = core
            .handle(AppEvent::ResetIndicatorConfirmed)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
            .unwrap();

        lifecycle.apply(
            &asset_store,
            &failing_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            reset,
        );

        assert_eq!(core.state().preferences, preferences);
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Ram)).unwrap(),
            original_bytes
        );
        assert_eq!(lifecycle.retained_cleanup.len(), 1);
        let blocked_snapshot = failing_store.blocked_snapshot.borrow().clone().unwrap();
        let effects = lifecycle.retry_cleanup(&mut core);
        assert!(effects.is_empty());
        assert_eq!(lifecycle.retained_cleanup.len(), 1);
        assert!(core.state().indicator_icon_error(MetricKind::Cpu).is_none());
        assert!(core
            .state()
            .indicator_icon_error(MetricKind::Ram)
            .unwrap()
            .contains("snapshot de Undo"));
        fs::set_permissions(&blocked_snapshot, fs::Permissions::from_mode(0o700)).unwrap();

        let effects = lifecycle.retry_cleanup(&mut core);

        assert!(effects.is_empty());
        assert!(lifecycle.retained_cleanup.is_empty());
    }

    #[test]
    fn undo_prepare_partial_rollback_waits_for_every_owner_and_recovery_save() {
        for metric_count in [1_usize, 2] {
            let directory = tempdir().unwrap();
            let icons = directory.path().join("icons");
            let asset_store = IconAssetStore::new(icons.clone());
            let preferences_store =
                PreferencesStore::new(directory.path().join("preferences.json"));
            let metrics = [MetricKind::Cpu, MetricKind::Ram]
                .into_iter()
                .take(metric_count)
                .collect::<Vec<_>>();
            let mut core = StatletCore::new();
            for (index, metric) in metrics.iter().enumerate() {
                let metadata = asset_store
                    .import_bytes(
                        *metric,
                        "original.png",
                        &png_bytes([0x10 + index as u8, 0x22, 0x33, 0xFF]),
                    )
                    .unwrap();
                core.handle(AppEvent::MetricPngImportFinished {
                    metric: *metric,
                    result: statlet::core::MetricPngImportResult::Imported(metadata),
                });
            }
            let original = core.state().preferences.clone();
            preferences_store.save(original).unwrap();
            let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
            let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
            let reset = core
                .handle(AppEvent::ResetIndicatorConfirmed)
                .into_iter()
                .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
                .unwrap();
            lifecycle.apply(
                &asset_store,
                &preferences_store,
                &mut schedule,
                Duration::ZERO,
                &mut core,
                reset,
            );
            for (index, metric) in metrics.iter().enumerate() {
                let metadata = asset_store
                    .import_bytes(
                        *metric,
                        "current.png",
                        &png_bytes([0xA0 + index as u8, 0xBB, 0xCC, 0xFF]),
                    )
                    .unwrap();
                core.handle(AppEvent::MetricPngImportFinished {
                    metric: *metric,
                    result: statlet::core::MetricPngImportResult::Imported(metadata),
                });
            }
            let current_preferences = core.state().preferences.clone();
            preferences_store.save(current_preferences.clone()).unwrap();
            let (current, undo, preferences) = core
                .handle(AppEvent::UndoIndicatorReset)
                .into_iter()
                .find_map(|effect| match effect {
                    AppEffect::PersistGlobalIndicatorUndo {
                        current,
                        undo,
                        preferences,
                    } => Some((current, undo, preferences)),
                    _ => None,
                })
                .unwrap();
            let optimistic_undo = core.state().preferences.clone();
            let snapshot = lifecycle.undo.take().unwrap();
            let transactions = super::begin_asset_removals(&asset_store, &metrics).unwrap();
            let blocked_metric = *metrics.last().unwrap();
            let blocked_name = match blocked_metric {
                MetricKind::Cpu => "cpu.png",
                MetricKind::Ram => "ram.png",
            };
            let blocked_transaction = fs::read_dir(&icons)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.is_dir()
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.contains(blocked_name))
                        && path.join("previous.png").exists()
                })
                .unwrap();
            fs::set_permissions(&blocked_transaction, fs::Permissions::from_mode(0o500)).unwrap();
            let failure = super::MetricPngTransaction::rollback(IdentifierResetTransaction::new(
                transactions,
            ))
            .unwrap_err();
            let preparation_message = failure.message.clone();
            let save_status = core.state().preferences_save_status;

            let effects = lifecycle.persist_undo_after_preparation(
                super::GlobalIndicatorUndoPreparationContext {
                    preferences_store: &preferences_store,
                    schedule: &mut schedule,
                    now: Duration::from_secs(1),
                    core: &mut core,
                },
                super::GlobalIndicatorUndoPreparationPlan {
                    undo: super::GlobalIndicatorUndoPlan {
                        current: current.clone(),
                        undo: undo.clone(),
                        preferences,
                    },
                    snapshot,
                    metrics: metrics.clone(),
                    preparation: Err(failure),
                },
            );

            assert!(effects.is_empty());
            assert_eq!(core.state().preferences, optimistic_undo);
            assert!(!core.state().can_undo_indicator_reset);
            assert_eq!(core.state().preferences_save_status, save_status);
            assert_eq!(preferences_store.load(), current_preferences);
            assert_eq!(lifecycle.retained_transactions.len(), 1);
            match &lifecycle.retained_transactions[0].completion {
                Some(super::GlobalRecoveryCompletion::Undo(failure)) => {
                    assert_eq!(failure.current, current);
                    assert_eq!(failure.undo, undo);
                    assert_eq!(failure.message, preparation_message);
                    assert_eq!(
                        failure.stage,
                        statlet::core::GlobalIndicatorUndoFailureStage::AssetPreparation
                    );
                }
                _ => panic!("prepare rollback must retain its Undo completion"),
            }

            let still_blocked = lifecycle.retry_transactions(&mut core);
            assert!(still_blocked.completions.is_empty());
            assert!(still_blocked.pending_transactions);
            assert_eq!(core.state().preferences, optimistic_undo);
            assert_eq!(core.state().preferences_save_status, save_status);

            fs::set_permissions(&blocked_transaction, fs::Permissions::from_mode(0o700)).unwrap();
            let recovered = lifecycle.retry_transactions(&mut core);
            assert_eq!(recovered.completions.len(), 1);
            assert!(!recovered.pending_transactions);
            assert_eq!(core.state().preferences, optimistic_undo);
            for (index, metric) in metrics.iter().enumerate() {
                assert_eq!(
                    fs::read(asset_store.path_for(*metric)).unwrap(),
                    png_bytes([0xA0 + index as u8, 0xBB, 0xCC, 0xFF])
                );
            }

            let blocked_parent = directory.path().join("not-a-directory");
            fs::write(&blocked_parent, b"blocking file").unwrap();
            let failing_store = PreferencesStore::new(blocked_parent.join("preferences.json"));
            let completion = recovered.completions.into_iter().next().unwrap();
            let failed_save = super::persist_recovery_completion(
                &failing_store,
                &mut schedule,
                Duration::from_secs(2),
                &mut core,
                completion,
            );
            assert!(failed_save.retry.is_some());
            assert_eq!(core.state().preferences, optimistic_undo);
            assert_eq!(preferences_store.load(), current_preferences);
            assert_eq!(schedule.pending_save(), Some(&current_preferences));

            let successful_save = super::persist_recovery_completion(
                &preferences_store,
                &mut schedule,
                Duration::from_secs(3),
                &mut core,
                failed_save.retry.unwrap(),
            );
            assert!(successful_save.retry.is_none());
            assert_eq!(core.state().preferences.indicator, current);
            assert_eq!(preferences_store.load(), current_preferences);
            assert!(core.state().can_undo_indicator_reset);
            assert_eq!(schedule.pending_save(), None);
        }
    }

    #[test]
    fn second_global_reset_replaces_and_cleans_the_previous_asset_snapshot() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let metadata = asset_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png_bytes([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let mut core = StatletCore::new();
        core.handle(AppEvent::MetricPngImportFinished {
            metric: MetricKind::Cpu,
            result: statlet::core::MetricPngImportResult::Imported(metadata),
        });
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();

        for _ in 0..2 {
            let reset = core
                .handle(AppEvent::ResetIndicatorConfirmed)
                .into_iter()
                .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
                .unwrap();
            lifecycle.apply(
                &asset_store,
                &preferences_store,
                &mut schedule,
                Duration::ZERO,
                &mut core,
                reset,
            );
        }

        assert!(!asset_store.path_for(MetricKind::Cpu).exists());
        assert_eq!(
            fs::read_dir(directory.path().join("icons"))
                .unwrap()
                .count(),
            0
        );
        assert!(core.state().can_undo_indicator_reset);
    }

    #[test]
    fn failed_undo_save_restores_replacement_bytes_runtime_and_undo_ownership() {
        let directory = tempdir().unwrap();
        let asset_store = IconAssetStore::new(directory.path().join("icons"));
        let preferences_store = PreferencesStore::new(directory.path().join("preferences.json"));
        let original_metadata = asset_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png_bytes([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let mut core = StatletCore::new();
        core.handle(AppEvent::MetricPngImportFinished {
            metric: MetricKind::Cpu,
            result: statlet::core::MetricPngImportResult::Imported(original_metadata),
        });
        let mut lifecycle = GlobalIndicatorAssetLifecycle::default();
        let mut schedule = statlet::runtime_schedule::RuntimeSchedule::new();
        let reset = core
            .handle(AppEvent::ResetIndicatorConfirmed)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorReset { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            reset,
        );
        let prepared = asset_store
            .prepare_bytes(
                MetricKind::Cpu,
                "replacement.png",
                &png_bytes([0xAA, 0xBB, 0xCC, 0xFF]),
            )
            .unwrap();
        let replacement_metadata = prepared.metadata().clone();
        let transaction = asset_store.begin_replace(prepared).unwrap();
        let replacement = core
            .handle(AppEvent::MetricPngImportFinished {
                metric: MetricKind::Cpu,
                result: statlet::core::MetricPngImportResult::Imported(replacement_metadata),
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
        super::persist_metric_png_change(
            &preferences_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            replacement.0,
            replacement.1,
            replacement.2,
            transaction,
        );
        let replacement_preferences = core.state().preferences.clone();
        let replacement_bytes = fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap();
        let blocker = directory.path().join("blocked-parent");
        fs::write(&blocker, b"not a directory").unwrap();
        let failing_store = PreferencesStore::new(blocker.join("preferences.json"));

        let undo = core
            .handle(AppEvent::UndoIndicatorReset)
            .into_iter()
            .find(|effect| matches!(effect, AppEffect::PersistGlobalIndicatorUndo { .. }))
            .unwrap();
        lifecycle.apply(
            &asset_store,
            &failing_store,
            &mut schedule,
            Duration::ZERO,
            &mut core,
            undo,
        );

        assert_eq!(core.state().preferences, replacement_preferences);
        assert_eq!(
            fs::read(asset_store.path_for(MetricKind::Cpu)).unwrap(),
            replacement_bytes
        );
        assert!(core.state().can_undo_indicator_reset);
        assert_eq!(
            core.state().preferences_save_status,
            PreferencesSaveStatus::Failed
        );
    }

    fn png_bytes(color: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(12, 12, Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn test_content_fingerprint(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }
}
