use std::cmp::Ordering;
use std::ffi::CString;
use std::mem;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use statlet::core::{MemoryPressure, SystemSnapshot};
use statlet::metrics::{detailed_memory_from_counters, MemoryReading, VmCounters};
use statlet::system_usage::{
    ProcessMemory, ProcessSampleCancellation, ProcessSampleOutcome, SamplingCycle,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacSystemSample {
    pub compact: SystemSnapshot,
    pub memory: MemoryReading,
}

#[derive(Default)]
struct CycleSampleCache<T> {
    entry: Option<(SamplingCycle, T)>,
}

impl<T: Copy> CycleSampleCache<T> {
    fn get_or_sample(&mut self, cycle: SamplingCycle, sample: impl FnOnce() -> T) -> T {
        if let Some((cached_cycle, value)) = self.entry {
            if cached_cycle == cycle {
                return value;
            }
        }
        let value = sample();
        self.entry = Some((cycle, value));
        value
    }
}

pub struct MacSampler {
    system: System,
    samples: CycleSampleCache<Option<MacSystemSample>>,
}

impl MacSampler {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            samples: CycleSampleCache::default(),
        }
    }

    pub fn prime_cpu(&mut self) {
        self.system.refresh_cpu_usage();
    }

    pub fn sample_in_cycle(&mut self, cycle: SamplingCycle) -> Option<MacSystemSample> {
        let system = &mut self.system;
        self.samples.get_or_sample(cycle, || sample_system(system))
    }

    pub fn sample_processes(cancellation: &ProcessSampleCancellation) -> ProcessSampleOutcome {
        if cancellation.is_cancelled() {
            return ProcessSampleOutcome::Cancelled;
        }
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        if cancellation.is_cancelled() {
            return ProcessSampleOutcome::Cancelled;
        }
        ProcessSampleOutcome::Available(
            select_top_process_ids(
                system
                    .processes()
                    .iter()
                    .map(|(pid, process)| (pid.as_u32(), process.memory())),
                20,
            )
            .into_iter()
            .filter_map(|(pid, memory_bytes)| {
                system
                    .processes()
                    .iter()
                    .find(|(candidate, _)| candidate.as_u32() == pid)
                    .map(|(_, process)| ProcessMemory {
                        pid,
                        name: process.name().to_string_lossy().into_owned(),
                        memory_bytes,
                    })
            })
            .collect(),
        )
    }
}

fn sample_system(system: &mut System) -> Option<MacSystemSample> {
    system.refresh_cpu_usage();
    system.refresh_memory();
    let memory = read_memory(system.total_memory(), system.used_swap())?;

    Some(MacSystemSample {
        compact: SystemSnapshot {
            cpu_percent: system.global_cpu_usage() as f64,
            ram_percent: memory.percent,
            memory_pressure: memory.pressure,
        },
        memory,
    })
}

fn select_top_process_ids<I>(candidates: I, limit: usize) -> Vec<(u32, u64)>
where
    I: IntoIterator<Item = (u32, u64)>,
{
    let mut top = Vec::with_capacity(limit);
    for candidate in candidates {
        if top.len() < limit {
            top.push(candidate);
            continue;
        }
        let Some((worst_index, worst)) = top
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| process_rank(left, right))
        else {
            continue;
        };
        if process_rank(&candidate, worst).is_gt() {
            top[worst_index] = candidate;
        }
    }
    top.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    top
}

fn process_rank(left: &(u32, u64), right: &(u32, u64)) -> Ordering {
    left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0))
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::select_top_process_ids;
    use super::CycleSampleCache;
    use statlet::system_usage::SamplingCycle;

    #[test]
    fn one_sampling_cycle_performs_one_physical_read_for_two_consumers() {
        let reads = Cell::new(0);
        let mut cache = CycleSampleCache::default();
        let cycle = SamplingCycle::new(7);
        let read = || {
            reads.set(reads.get() + 1);
            42_u8
        };

        assert_eq!(cache.get_or_sample(cycle, read), 42);
        assert_eq!(cache.get_or_sample(cycle, read), 42);
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn process_candidates_are_bounded_and_sorted_before_names_are_allocated() {
        let candidates = (0..25).map(|pid| (pid, u64::from(pid) * 100));

        let top = select_top_process_ids(candidates, 20);

        assert_eq!(top.len(), 20);
        assert_eq!(top.first(), Some(&(24, 2_400)));
        assert_eq!(top.last(), Some(&(5, 500)));
        assert_eq!(
            select_top_process_ids([(9, 500), (3, 500), (7, 100)], 2),
            vec![(3, 500), (9, 500)]
        );
    }
}
