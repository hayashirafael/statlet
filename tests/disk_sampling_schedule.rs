use std::time::Duration;

use statlet::disk::DiskSamplingSchedule;

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

#[test]
fn schedule_samples_immediately_then_once_per_minute_only_while_enabled() {
    let mut schedule = DiskSamplingSchedule::new();

    assert!(!schedule.take_due(seconds(0)));
    schedule.set_enabled(true, seconds(10));
    assert!(schedule.take_due(seconds(10)));
    assert!(!schedule.take_due(seconds(69)));
    assert!(schedule.take_due(seconds(70)));

    schedule.set_enabled(false, seconds(71));
    assert!(!schedule.take_due(seconds(500)));
}

#[test]
fn delayed_wakeup_takes_one_sample_without_catch_up_burst() {
    let mut schedule = DiskSamplingSchedule::new();
    schedule.set_enabled(true, seconds(0));
    assert!(schedule.take_due(seconds(0)));

    assert!(schedule.take_due(seconds(500)));
    assert!(!schedule.take_due(seconds(500)));
    assert!(!schedule.take_due(seconds(559)));
    assert!(schedule.take_due(seconds(560)));
}

#[test]
fn a_backward_wall_clock_change_reschedules_immediately_without_stalling() {
    let mut schedule = DiskSamplingSchedule::new();
    schedule.set_enabled(true, seconds(100));
    assert!(schedule.take_due(seconds(100)));
    assert!(!schedule.take_due(seconds(120)));

    assert!(schedule.take_due(seconds(50)));
    assert!(!schedule.take_due(seconds(109)));
    assert!(schedule.take_due(seconds(110)));
}

#[test]
fn disabled_disk_has_no_deadline_and_enabled_disk_reports_remaining_time() {
    let mut schedule = DiskSamplingSchedule::new();

    assert_eq!(schedule.remaining(seconds(0)), None);
    schedule.set_enabled(true, seconds(0));
    assert_eq!(schedule.remaining(seconds(0)), Some(Duration::ZERO));
}
