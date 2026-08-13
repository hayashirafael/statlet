use std::ffi::CString;
use std::mem;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use statlet::core::{MemoryPressure, SystemSnapshot};
use statlet::metrics::{detailed_memory_from_counters, MemoryReading, VmCounters};
use statlet::stats::ProcessMemory;

pub struct MacSystemSample {
    pub compact: SystemSnapshot,
    pub memory: MemoryReading,
}

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

    pub fn sample(&mut self) -> Option<MacSystemSample> {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        let memory = read_memory(self.system.total_memory(), self.system.used_swap())?;

        Some(MacSystemSample {
            compact: SystemSnapshot {
                cpu_percent: self.system.global_cpu_usage() as f64,
                ram_percent: memory.percent,
                memory_pressure: memory.pressure,
            },
            memory,
        })
    }

    pub fn sample_processes(&mut self) -> Vec<ProcessMemory> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        self.system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessMemory {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                memory_bytes: process.memory(),
            })
            .collect()
    }
}

fn read_memory(total_bytes: u64, swap_used_bytes: u64) -> Option<MemoryReading> {
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

    Some(detailed_memory_from_counters(
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
        swap_used_bytes,
        read_memory_pressure()?,
    ))
}

fn read_memory_pressure() -> Option<MemoryPressure> {
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
        return None;
    }

    MemoryPressure::try_from(level).ok()
}
