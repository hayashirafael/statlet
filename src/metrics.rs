//! System metric domain types and platform conversion functions.

use crate::core::MemoryPressure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmCounters {
    pub active: u64,
    pub inactive: u64,
    pub speculative: u64,
    pub wired: u64,
    pub compressed: u64,
    pub purgeable: u64,
    pub external: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemoryReading {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
    pub pressure: MemoryPressure,
}

pub fn memory_from_counters(
    total_bytes: u64,
    page_size: u64,
    counters: VmCounters,
    pressure: MemoryPressure,
) -> MemoryReading {
    let used_pages = counters
        .active
        .saturating_add(counters.inactive)
        .saturating_add(counters.speculative)
        .saturating_add(counters.wired)
        .saturating_add(counters.compressed)
        .saturating_sub(counters.purgeable)
        .saturating_sub(counters.external);
    let used_bytes = used_pages.saturating_mul(page_size).min(total_bytes);
    let percent = if total_bytes == 0 {
        0.0
    } else {
        used_bytes as f64 / total_bytes as f64 * 100.0
    };

    MemoryReading {
        used_bytes,
        total_bytes,
        percent,
        pressure,
    }
}
