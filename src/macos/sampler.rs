use std::ffi::CString;
use std::mem;

use sysinfo::System;

use statlet::core::{MemoryPressure, SystemSnapshot};
use statlet::metrics::{memory_from_counters, MemoryReading, VmCounters};

pub struct MacSampler {
    system: System,
}

impl MacSampler {
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }

    pub fn prime_cpu(&mut self) {
        self.system.refresh_cpu_usage();
    }

    pub fn sample(&mut self) -> Option<SystemSnapshot> {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        let memory = read_memory(self.system.total_memory())?;

        Some(SystemSnapshot {
            cpu_percent: self.system.global_cpu_usage() as f64,
            ram_percent: memory.percent,
            memory_pressure: memory.pressure,
        })
    }
}

fn read_memory(total_bytes: u64) -> Option<MemoryReading> {
    let mut stats = unsafe { mem::zeroed::<libc::vm_statistics64>() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    #[allow(deprecated)]
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

    Some(memory_from_counters(
        total_bytes,
        page_size as u64,
        VmCounters {
            active: u64::from(stats.active_count),
            inactive: u64::from(stats.inactive_count),
            speculative: u64::from(stats.speculative_count),
            wired: u64::from(stats.wire_count),
            compressed: u64::from(stats.compressor_page_count),
            purgeable: u64::from(stats.purgeable_count),
            external: u64::from(stats.external_page_count),
        },
        read_memory_pressure(),
    ))
}

fn read_memory_pressure() -> MemoryPressure {
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
