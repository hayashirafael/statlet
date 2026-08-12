//! Statlet macOS runtime.
//!
//! Event-loop structure derived and modified from featherbar commit 90ab504,
//! licensed under Apache-2.0.

mod macos;

use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use objc2::MainThreadMarker;
use statlet::core::{AppEffect, AppEvent, Preferences, PreferencesSaveResult, StatletCore};
use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};
use statlet::disk::DiskSamplingSchedule;
use statlet::history::{History, HistoryStore};
use statlet::indicator_preferences::MetricsRefreshInterval;
use statlet::metrics_schedule::MetricsSamplingSchedule;
use statlet::mole::{MoleDetection, MoleDetector, MoleInstallation, MoleStatus};
use statlet::preferences::PreferencesStore;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use macos::notifications::NotificationManager;
use macos::renderer::Renderer;
use macos::sampler::MacSampler;
use macos::windows::WindowManager;
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
            let sampling_effects =
                poll_due_and_render(&mut runtime, &mut core, &renderer, button.as_deref());
            let _ = runtime.apply_effects(&sampling_effects, &mut core);
            set_next_wakeup(control_flow, &runtime.samplers);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            if button.is_none() {
                let marker = MainThreadMarker::new().expect("main-thread event loop");
                button = macos::renderer::status_button(marker);
            }
            let sampling_effects =
                poll_due_and_render(&mut runtime, &mut core, &renderer, button.as_deref());
            let _ = runtime.apply_effects(&sampling_effects, &mut core);
            set_next_wakeup(control_flow, &runtime.samplers);
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
            };
            if runtime.apply_effects(&effects, &mut core) {
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
            let _ = runtime.apply_effects(&effects, &mut core);
        }
        _ => {}
    });
}

fn poll_due_and_render(
    runtime: &mut RuntimeAdapters,
    core: &mut StatletCore,
    renderer: &Renderer,
    button: Option<&objc2_app_kit::NSStatusBarButton>,
) -> Vec<AppEffect> {
    objc2::rc::autoreleasepool(|_| {
        let effects = runtime.samplers.poll_due(core);
        if let Some(button) = button {
            renderer.set_status(button, &core.state().status);
        }
        effects
    })
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
            review_space_item,
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

    fn apply_effects(&mut self, effects: &[AppEffect], core: &mut StatletCore) -> bool {
        let mut should_quit = false;
        let mut pending = effects.iter().cloned().collect::<VecDeque<_>>();
        while let Some(effect) = pending.pop_front() {
            match effect {
                AppEffect::RedrawIndicator => {}
                AppEffect::SetMetricsSamplingInterval(interval) => {
                    self.samplers.reschedule_metrics(interval);
                }
                AppEffect::ShowWindow(kind) => {
                    if let Some(windows) = &mut self.windows {
                        windows.show(kind, core.state(), &self.history);
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

    use statlet::core::{Preferences, PreferencesSaveStatus};
    use statlet::indicator_preferences::MetricsRefreshInterval;
    use tempfile::tempdir;

    use super::{save_preferences, PreferencesStore, RuntimeSamplers, StatletCore};

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
