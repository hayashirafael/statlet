//! Statlet macOS runtime.
//!
//! Event-loop structure derived and modified from featherbar commit 90ab504,
//! licensed under Apache-2.0.

mod macos;

use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use objc2::MainThreadMarker;
use statlet::core::{AppEffect, AppEvent, StatletCore};
use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};
use statlet::disk::DiskSamplingSchedule;
use statlet::history::{History, HistoryStore};
use statlet::mole::{MoleDetection, MoleDetector, MoleInstallation, MoleStatus};
use statlet::preferences::PreferencesStore;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use macos::gpu::MacGpuSampler;
use macos::notifications::NotificationManager;
use macos::renderer::Renderer;
use macos::sampler::MacSampler;
use macos::windows::WindowManager;
use macos::RuntimeEvent;
use statlet::stats::{
    ProcessSampleCancellation, ProcessSampleCompletion, SystemUsageModel,
    SystemUsageRenderCoalescer, SystemUsageSamplingCoordinator, SystemUsageSamplingPorts,
    SystemUsageSection,
};

const METRICS_REFRESH: Duration = Duration::from_secs(2);

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
    let history = history_store.load();
    let initial_preferences = preferences_store.load();
    let (mut core, startup_effects) = StatletCore::with_preferences(initial_preferences);
    let mut runtime = RuntimeAdapters::new(
        preferences_store,
        history_store,
        history,
        review_space_item,
        proxy.clone(),
    );
    let renderer = Renderer::new();
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
            runtime.initialize_native(marker, proxy.clone());
            let _ = proxy.send_event(RuntimeEvent::App(AppEvent::ApplicationLaunched));
            let _ = runtime.apply_effects(&startup_effects, &mut core);
            let disk_effects = runtime.refresh(&mut core, &renderer, button.as_deref());
            let _ = runtime.apply_effects(&disk_effects, &mut core);
            *control_flow = ControlFlow::WaitUntil(Instant::now() + METRICS_REFRESH);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            if button.is_none() {
                let marker = MainThreadMarker::new().expect("main-thread event loop");
                button = macos::renderer::status_button(marker);
            }
            let disk_effects = runtime.refresh(&mut core, &renderer, button.as_deref());
            let _ = runtime.apply_effects(&disk_effects, &mut core);
            *control_flow = ControlFlow::WaitUntil(Instant::now() + METRICS_REFRESH);
        }
        Event::UserEvent(runtime_event) => {
            let effects = match runtime_event {
                RuntimeEvent::App(app_event) => core.handle(app_event),
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
            };
            if runtime.apply_effects(&effects, &mut core) {
                *control_flow = ControlFlow::Exit;
            }
        }
        Event::Reopen {
            has_visible_windows,
            ..
        } => {
            let effects = core.handle(AppEvent::ApplicationReopened {
                has_visible_windows,
            });
            let _ = runtime.apply_effects(&effects, &mut core);
        }
        _ => {}
    });
}

struct RuntimeAdapters {
    preferences_store: PreferencesStore,
    history_store: HistoryStore,
    history: History,
    windows: Option<WindowManager>,
    samplers: RuntimeSamplers,
    mole: RuntimeMole,
    notifications: Option<NotificationManager>,
    review_space_item: MenuItem,
    system_usage_rendering: SystemUsageRenderCoalescer,
}

impl RuntimeAdapters {
    fn new(
        preferences_store: PreferencesStore,
        history_store: HistoryStore,
        history: History,
        review_space_item: MenuItem,
        proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
    ) -> Self {
        Self {
            preferences_store,
            history_store,
            history,
            windows: None,
            samplers: RuntimeSamplers::new(proxy.clone()),
            mole: RuntimeMole::new(proxy),
            notifications: None,
            review_space_item,
            system_usage_rendering: SystemUsageRenderCoalescer::new(),
        }
    }

    fn initialize_native(
        &mut self,
        marker: MainThreadMarker,
        proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
    ) {
        self.windows = Some(WindowManager::new(marker, proxy.clone()));
        self.notifications = NotificationManager::new(marker, proxy);
    }

    fn refresh(
        &mut self,
        core: &mut StatletCore,
        renderer: &Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
    ) -> Vec<AppEffect> {
        let system_usage_visible = self
            .windows
            .as_ref()
            .is_some_and(WindowManager::system_usage_visible);
        let process_interaction_active = self.system_usage_process_interaction_active();
        let system_usage_visibility_generation = self.system_usage_visibility_generation();
        let effects = self.samplers.refresh(
            core,
            renderer,
            button,
            system_usage_visible,
            process_interaction_active,
            system_usage_visibility_generation,
        );
        self.update_system_usage_window(core.state().system_usage_section);
        effects
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

    fn apply_effects(&mut self, effects: &[AppEffect], core: &mut StatletCore) -> bool {
        let mut should_quit = false;
        let mut pending = effects.iter().copied().collect::<VecDeque<_>>();
        while let Some(effect) = pending.pop_front() {
            match effect {
                AppEffect::ShowWindow(kind) => {
                    if let Some(windows) = &mut self.windows {
                        windows.show(kind, core.state(), &self.history);
                    }
                    if kind == statlet::core::WindowKind::SystemUsage {
                        self.system_usage_rendering.reset();
                    }
                }
                AppEffect::SavePreferences(preferences) => {
                    if let Err(error) = self.preferences_store.save(preferences) {
                        eprintln!("Statlet could not save preferences: {error}");
                    }
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
        self.update_system_usage_window(core.state().system_usage_section);
        should_quit
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
    disk: StartupVolumeSampler,
    disk_schedule: DiskSamplingSchedule,
    clock: ContinuousClock,
    gpu: MacGpuSampler,
    system_usage: SystemUsageModel,
    system_usage_sampling: SystemUsageSamplingCoordinator,
    process_proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>,
}

impl RuntimeSamplers {
    fn new(process_proxy: tao::event_loop::EventLoopProxy<RuntimeEvent>) -> Self {
        let mut metrics = MacSampler::new();
        metrics.prime_cpu();
        Self {
            metrics,
            disk: StartupVolumeSampler::new(),
            disk_schedule: DiskSamplingSchedule::new(),
            clock: ContinuousClock::new().expect("initialize the macOS continuous clock"),
            gpu: MacGpuSampler::new(),
            system_usage: SystemUsageModel::new(),
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
        self.system_usage_sampling.set_visible(now, visible);
    }

    fn refresh(
        &mut self,
        core: &mut StatletCore,
        renderer: &Renderer,
        button: Option<&objc2_app_kit::NSStatusBarButton>,
        system_usage_visible: bool,
        process_interaction_active: bool,
        system_usage_visibility_generation: u64,
    ) -> Vec<AppEffect> {
        objc2::rc::autoreleasepool(|_| {
            let now = self.clock.now();
            self.system_usage.set_visible(system_usage_visible);
            self.system_usage
                .apply_deferred_processes(now, process_interaction_active);
            match self.metrics.sample() {
                Some(snapshot) => {
                    core.handle(AppEvent::MetricsSample(snapshot.compact));
                    self.system_usage.record_memory(now, Ok(snapshot.memory));
                }
                None => self.system_usage.record_memory(now, Err(())),
            }
            let mut ports = RuntimeSystemUsagePorts {
                gpu: &mut self.gpu,
                process_proxy: &self.process_proxy,
                visibility_generation: system_usage_visibility_generation,
            };
            if let Some(gpu) =
                self.system_usage_sampling
                    .collect_if_visible(now, system_usage_visible, &mut ports)
            {
                self.system_usage.record_gpu(now, gpu);
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
            let state = core.state();
            if let Some(button) = button {
                renderer.set_status(button, &state.status);
            }
            effects
        })
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
    process_proxy: &'a tao::event_loop::EventLoopProxy<RuntimeEvent>,
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
        let proxy = self.process_proxy.clone();
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
                self.process_proxy
                    .send_event(RuntimeEvent::ProcessesSampled {
                        generation,
                        visibility_generation,
                        outcome: statlet::stats::ProcessSampleOutcome::Failed,
                    })
                    .is_ok()
            })
    }
}
