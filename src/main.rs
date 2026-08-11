//! PROTOTYPE — runtime feasibility only. Do not merge into production.
//!
//! Event-loop structure derived from featherbar, Apache-2.0:
//! https://github.com/nim444/featherbar/tree/90ab504b025db15665ce5d97b8ae4d4cdeb47dc3

mod two_line;

use std::ffi::CString;
use std::mem;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use sysinfo::System;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};
use two_line::{Level, Renderer, Seg};

const REFRESH: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug)]
struct MemorySample {
    used_bytes: u64,
    total_bytes: u64,
    pressure: MemoryPressure,
}

impl MemorySample {
    fn percent(self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.used_bytes as f64 / self.total_bytes as f64 * 100.0
    }
}

struct Sampler {
    system: System,
}

impl Sampler {
    fn new() -> Self {
        Self {
            system: System::new(),
        }
    }

    fn prime_cpu(&mut self) {
        self.system.refresh_cpu_usage();
    }

    fn sample(&mut self) -> (f64, MemorySample) {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        let cpu = self.system.global_cpu_usage() as f64;
        let memory = sample_memory(self.system.total_memory()).unwrap_or(MemorySample {
            used_bytes: 0,
            total_bytes: self.system.total_memory(),
            pressure: MemoryPressure::Normal,
        });
        (cpu, memory)
    }
}

fn sample_memory(total_bytes: u64) -> Option<MemorySample> {
    let mut stats = unsafe { mem::zeroed::<libc::vm_statistics64>() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let result = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            &mut stats as *mut libc::vm_statistics64 as *mut _,
            &mut count,
        )
    };
    if result != libc::KERN_SUCCESS {
        return None;
    }

    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    let page_size = page_size as u64;

    // Matches the user-visible model used by Stats:
    // apps + wired + compressed, excluding purgeable and external cache.
    let used_pages = u64::from(stats.active_count)
        .saturating_add(u64::from(stats.inactive_count))
        .saturating_add(u64::from(stats.speculative_count))
        .saturating_add(u64::from(stats.wire_count))
        .saturating_add(u64::from(stats.compressor_page_count))
        .saturating_sub(u64::from(stats.purgeable_count))
        .saturating_sub(u64::from(stats.external_page_count));

    Some(MemorySample {
        used_bytes: used_pages.saturating_mul(page_size),
        total_bytes,
        pressure: memory_pressure(),
    })
}

fn memory_pressure() -> MemoryPressure {
    let name = CString::new("kern.memorystatus_vm_pressure_level").expect("static sysctl name");
    let mut level: libc::c_int = 0;
    let mut size = mem::size_of_val(&level);
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut level as *mut libc::c_int as *mut _,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return MemoryPressure::Normal;
    }

    match level {
        2 => MemoryPressure::Warning,
        4 => MemoryPressure::Critical,
        _ => MemoryPressure::Normal,
    }
}

fn cpu_level(percent: f64) -> Level {
    match percent {
        value if value < 40.0 => Level::Good,
        value if value < 70.0 => Level::Warn,
        _ => Level::Crit,
    }
}

fn memory_level(pressure: MemoryPressure) -> Level {
    match pressure {
        MemoryPressure::Normal => Level::Good,
        MemoryPressure::Warning => Level::Warn,
        MemoryPressure::Critical => Level::Crit,
    }
}

fn segments(label: &str, percent: f64, level: Level) -> Vec<Seg> {
    vec![
        Seg::new(label, Level::Neutral),
        Seg::new(format!("{percent:>3.0}%"), level),
    ]
}

fn render_sample(
    sampler: &mut Sampler,
    renderer: &Renderer,
    button: Option<&objc2_app_kit::NSStatusBarButton>,
    tray: Option<&TrayIcon>,
) {
    let (cpu, memory) = sampler.sample();
    let ram = memory.percent();
    let top = segments("C", cpu, cpu_level(cpu));
    let bottom = segments("R", ram, memory_level(memory.pressure));

    eprintln!(
        "cpu={cpu:.1}% ram={ram:.1}% pressure={:?} used={} total={}",
        memory.pressure, memory.used_bytes, memory.total_bytes
    );

    if let Some(button) = button {
        renderer.set_title(button, &top, &bottom);
    } else if let Some(tray) = tray {
        tray.set_title(Some(format!("C {cpu:.0}% · R {ram:.0}%")));
    }
}

fn main() {
    let mut event_loop = EventLoopBuilder::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let quit = MenuItem::new("Quit prototype", true, None);
    let quit_id: MenuId = quit.id().clone();
    let menu = Menu::new();
    menu.append(&quit).expect("prototype menu");

    let mut sampler = Sampler::new();
    sampler.prime_cpu();
    let renderer = Renderer::new();
    let mut tray: Option<TrayIcon> = None;
    let mut button = None;

    event_loop.run(move |event, _target, control_flow| match event {
        Event::NewEvents(StartCause::Init) => {
            tray = Some(
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .build()
                    .expect("prototype status item"),
            );
            let marker = MainThreadMarker::new().expect("main-thread event loop");
            button = two_line::status_button(marker);
            objc2::rc::autoreleasepool(|_| {
                render_sample(&mut sampler, &renderer, button.as_deref(), tray.as_ref())
            });
            *control_flow = ControlFlow::WaitUntil(Instant::now() + REFRESH);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == quit_id {
                    *control_flow = ControlFlow::Exit;
                    return;
                }
            }
            objc2::rc::autoreleasepool(|_| {
                render_sample(&mut sampler, &renderer, button.as_deref(), tray.as_ref())
            });
            *control_flow = ControlFlow::WaitUntil(Instant::now() + REFRESH);
        }
        _ => {}
    });
}
