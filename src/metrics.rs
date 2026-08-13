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
    pub app_bytes: u64,
    pub wired_bytes: u64,
    pub compressed_bytes: u64,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub cached_bytes: u64,
    pub swap_used_bytes: u64,
    pub percent: f64,
    pub pressure: MemoryPressure,
}

pub fn memory_from_counters(
    total_bytes: u64,
    page_size: u64,
    counters: VmCounters,
    pressure: MemoryPressure,
) -> MemoryReading {
    detailed_memory_from_counters(total_bytes, page_size, counters, 0, pressure)
}

pub fn detailed_memory_from_counters(
    total_bytes: u64,
    page_size: u64,
    counters: VmCounters,
    swap_used_bytes: u64,
    pressure: MemoryPressure,
) -> MemoryReading {
    let app_pages = counters
        .active
        .saturating_add(counters.inactive)
        .saturating_add(counters.speculative)
        .saturating_sub(counters.purgeable)
        .saturating_sub(counters.external);
    let app_bytes = app_pages.saturating_mul(page_size).min(total_bytes);
    let wired_bytes = counters
        .wired
        .saturating_mul(page_size)
        .min(total_bytes.saturating_sub(app_bytes));
    let compressed_bytes = counters.compressed.saturating_mul(page_size).min(
        total_bytes
            .saturating_sub(app_bytes)
            .saturating_sub(wired_bytes),
    );
    let used_bytes = app_bytes
        .saturating_add(wired_bytes)
        .saturating_add(compressed_bytes)
        .min(total_bytes);
    let available_bytes = total_bytes.saturating_sub(used_bytes);
    let cached_bytes = counters
        .external
        .saturating_add(counters.purgeable)
        .saturating_mul(page_size)
        .min(available_bytes);
    let percent = if total_bytes == 0 {
        0.0
    } else {
        used_bytes as f64 / total_bytes as f64 * 100.0
    };

    MemoryReading {
        app_bytes,
        wired_bytes,
        compressed_bytes,
        used_bytes,
        total_bytes,
        available_bytes,
        cached_bytes,
        swap_used_bytes,
        percent,
        pressure,
    }
}
