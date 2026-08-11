//! Statlet macOS runtime.
//!
//! Event-loop structure derived and modified from featherbar commit 90ab504,
//! licensed under Apache-2.0.

mod macos;

use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use statlet::core::{AppEvent, StatletCore};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use macos::renderer::Renderer;
use macos::sampler::MacSampler;

const METRICS_REFRESH: Duration = Duration::from_secs(2);

fn main() {
    let mut event_loop = EventLoopBuilder::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let quit = MenuItem::new("Sair", true, None);
    let quit_id: MenuId = quit.id().clone();
    let menu = Menu::new();
    menu.append(&quit).expect("build menu");

    let mut core = StatletCore::new();
    let mut sampler = MacSampler::new();
    sampler.prime_cpu();
    let renderer = Renderer::new();
    let mut _tray: Option<TrayIcon> = None;
    let mut button = None;

    event_loop.run(move |event, _target, control_flow| match event {
        Event::NewEvents(StartCause::Init) => {
            _tray = Some(
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .build()
                    .expect("create status item"),
            );
            let marker = MainThreadMarker::new().expect("main-thread event loop");
            button = macos::renderer::status_button(marker);
            redraw(&mut core, &mut sampler, &renderer, button.as_deref());
            *control_flow = ControlFlow::WaitUntil(Instant::now() + METRICS_REFRESH);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == quit_id {
                    *control_flow = ControlFlow::Exit;
                    return;
                }
            }
            if button.is_none() {
                let marker = MainThreadMarker::new().expect("main-thread event loop");
                button = macos::renderer::status_button(marker);
            }
            redraw(&mut core, &mut sampler, &renderer, button.as_deref());
            *control_flow = ControlFlow::WaitUntil(Instant::now() + METRICS_REFRESH);
        }
        _ => {}
    });
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
        let state = core.handle(AppEvent::MetricsSample(snapshot));
        if let Some(button) = button {
            renderer.set_status(button, &state.status);
        }
    });
}
