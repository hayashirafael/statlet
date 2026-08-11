//! Statlet macOS runtime.
//!
//! Event-loop structure derived and modified from featherbar commit 90ab504,
//! licensed under Apache-2.0.

mod macos;

use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use statlet::core::{AppEffect, AppEvent, StatletCore};
use statlet::preferences::PreferencesStore;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use macos::renderer::Renderer;
use macos::sampler::MacSampler;
use macos::windows::{RuntimeEvent, WindowManager};

const METRICS_REFRESH: Duration = Duration::from_secs(2);

fn main() {
    let mut event_loop = EventLoopBuilder::<RuntimeEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();

    let preferences_item = MenuItem::new("Preferências…", true, None);
    let preferences_id: MenuId = preferences_item.id().clone();
    let history_item = MenuItem::new("Histórico…", true, None);
    let history_id: MenuId = history_item.id().clone();
    let quit = MenuItem::new("Sair", true, None);
    let quit_id: MenuId = quit.id().clone();
    let menu = Menu::new();
    menu.append(&preferences_item).expect("build menu");
    menu.append(&history_item).expect("build menu");
    menu.append(&quit).expect("build menu");

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let runtime_event = if event.id == preferences_id {
            Some(RuntimeEvent(AppEvent::OpenPreferences))
        } else if event.id == history_id {
            Some(RuntimeEvent(AppEvent::OpenHistory))
        } else if event.id == quit_id {
            Some(RuntimeEvent(AppEvent::Quit))
        } else {
            None
        };
        if let Some(event) = runtime_event {
            let _ = menu_proxy.send_event(event);
        }
    }));

    let preferences_store = PreferencesStore::for_current_user()
        .expect("resolve the current user's preferences directory");
    let initial_preferences = preferences_store.load();
    let (mut core, startup_effects) = StatletCore::with_preferences(initial_preferences);
    let mut sampler = MacSampler::new();
    sampler.prime_cpu();
    let renderer = Renderer::new();
    // tray-icon removes the status item when its owner is dropped.
    let mut _retained_tray: Option<TrayIcon> = None;
    let mut button = None;
    let mut windows = None;
    let mut disk_sampling_enabled = false;

    event_loop.run(move |event, _target, control_flow| match event {
        Event::NewEvents(StartCause::Init) => {
            _retained_tray = Some(
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .build()
                    .expect("create status item"),
            );
            let marker = MainThreadMarker::new().expect("main-thread event loop");
            button = macos::renderer::status_button(marker);
            windows = Some(WindowManager::new(marker, proxy.clone()));
            let _ = apply_effects(
                &startup_effects,
                &core,
                &preferences_store,
                windows.as_mut(),
                &mut disk_sampling_enabled,
            );
            redraw(&mut core, &mut sampler, &renderer, button.as_deref());
            *control_flow = ControlFlow::WaitUntil(Instant::now() + METRICS_REFRESH);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            if button.is_none() {
                let marker = MainThreadMarker::new().expect("main-thread event loop");
                button = macos::renderer::status_button(marker);
            }
            redraw(&mut core, &mut sampler, &renderer, button.as_deref());
            *control_flow = ControlFlow::WaitUntil(Instant::now() + METRICS_REFRESH);
        }
        Event::UserEvent(RuntimeEvent(app_event)) => {
            let effects = core.handle(app_event);
            if apply_effects(
                &effects,
                &core,
                &preferences_store,
                windows.as_mut(),
                &mut disk_sampling_enabled,
            ) {
                *control_flow = ControlFlow::Exit;
            }
        }
        _ => {}
    });
}

fn apply_effects(
    effects: &[AppEffect],
    core: &StatletCore,
    preferences_store: &PreferencesStore,
    mut windows: Option<&mut WindowManager>,
    disk_sampling_enabled: &mut bool,
) -> bool {
    let mut should_quit = false;
    for effect in effects {
        match effect {
            AppEffect::ShowWindow(kind) => {
                if let Some(windows) = windows.as_deref_mut() {
                    windows.show(*kind, core.state().preferences);
                }
            }
            AppEffect::SavePreferences(preferences) => {
                if let Err(error) = preferences_store.save(*preferences) {
                    eprintln!("Statlet could not save preferences: {error}");
                }
                if let Some(windows) = windows.as_deref_mut() {
                    windows.update_preferences(*preferences);
                }
            }
            AppEffect::SetDiskSamplingEnabled(enabled) => {
                *disk_sampling_enabled = *enabled;
            }
            AppEffect::Quit => should_quit = true,
        }
    }
    should_quit
}

fn redraw(
    core: &mut StatletCore,
    sampler: &mut MacSampler,
    renderer: &Renderer,
    button: Option<&objc2_app_kit::NSStatusBarButton>,
) {
    let Some(snapshot) = sampler.sample() else {
        return;
    };

    objc2::rc::autoreleasepool(|_| {
        core.handle(AppEvent::MetricsSample(snapshot));
        let state = core.state();
        if let Some(button) = button {
            renderer.set_status(button, &state.status);
        }
    });
}
