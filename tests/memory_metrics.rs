use statlet::core::MemoryPressure;
use statlet::metrics::{memory_from_counters, VmCounters};

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
