#![cfg(target_os = "macos")]

use std::time::Duration;

use statlet::disk::macos::{ContinuousClock, StartupVolumeSampler};

#[test]
fn startup_volume_reports_total_and_important_usage_capacity() {
    let observed_at = Duration::from_secs(42);

    let observation = StartupVolumeSampler::new().sample(observed_at).unwrap();

    assert!(observation.total_bytes() > 0);
    assert!(observation.available_bytes() <= observation.total_bytes());
    assert_eq!(observation.observed_at(), observed_at);
    assert!((0.0..=100.0).contains(&observation.occupied_percent()));
}

#[test]
fn continuous_clock_never_moves_backwards() {
    let clock = ContinuousClock::new().unwrap();

    let first = clock.now();
    let second = clock.now();

    assert!(second >= first);
}
