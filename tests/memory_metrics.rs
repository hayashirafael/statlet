use statlet::core::MemoryPressure;
use statlet::metrics::{detailed_memory_from_counters, memory_from_counters, VmCounters};

#[test]
fn memory_excludes_reclaimable_cache_from_the_user_visible_value() {
    let reading = memory_from_counters(
        16_000,
        100,
        VmCounters {
            active: 20,
            inactive: 10,
            speculative: 2,
            wired: 5,
            compressed: 4,
            purgeable: 3,
            external: 6,
        },
        MemoryPressure::Warning,
    );

    assert_eq!(reading.used_bytes, 3_200);
    assert_eq!(reading.total_bytes, 16_000);
    assert_eq!(reading.percent, 20.0);
    assert_eq!(reading.pressure, MemoryPressure::Warning);
}

#[test]
fn detailed_memory_snapshot_preserves_components_without_counting_cache_as_used() {
    let reading = detailed_memory_from_counters(
        16_000,
        100,
        VmCounters {
            active: 20,
            inactive: 10,
            speculative: 2,
            wired: 5,
            compressed: 4,
            purgeable: 3,
            external: 6,
        },
        700,
        MemoryPressure::Warning,
    );

    assert_eq!(reading.app_bytes, 2_300);
    assert_eq!(reading.wired_bytes, 500);
    assert_eq!(reading.compressed_bytes, 400);
    assert_eq!(reading.used_bytes, 3_200);
    assert_eq!(reading.available_bytes, 12_800);
    assert_eq!(reading.cached_bytes, 900);
    assert_eq!(reading.swap_used_bytes, 700);
    assert_eq!(reading.percent, 20.0);
    assert_eq!(reading.pressure, MemoryPressure::Warning);
}

#[test]
fn detailed_memory_components_never_exceed_physical_memory() {
    let reading = detailed_memory_from_counters(
        1_000,
        100,
        VmCounters {
            active: 20,
            inactive: 0,
            speculative: 0,
            wired: 8,
            compressed: 4,
            purgeable: 0,
            external: 0,
        },
        0,
        MemoryPressure::Critical,
    );

    assert_eq!(reading.used_bytes, 1_000);
    assert_eq!(
        reading.app_bytes + reading.wired_bytes + reading.compressed_bytes,
        reading.used_bytes
    );
    assert_eq!(reading.available_bytes, 0);
    assert_eq!(reading.cached_bytes, 0);
}

#[test]
fn unknown_memory_pressure_is_not_reported_as_normal() {
    assert_eq!(MemoryPressure::try_from(1), Ok(MemoryPressure::Normal));
    assert_eq!(MemoryPressure::try_from(2), Ok(MemoryPressure::Warning));
    assert_eq!(MemoryPressure::try_from(4), Ok(MemoryPressure::Critical));
    assert!(MemoryPressure::try_from(0).is_err());
    assert!(MemoryPressure::try_from(99).is_err());
}
