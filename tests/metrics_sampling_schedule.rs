use std::time::Duration;

use statlet::indicator_preferences::MetricsRefreshInterval;
use statlet::metrics_schedule::MetricsSamplingSchedule;

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn interval(value: u8) -> MetricsRefreshInterval {
    MetricsRefreshInterval::try_from(value).unwrap()
}

#[test]
fn schedule_is_due_now_then_uses_the_default_two_seconds() {
    let now = seconds(10);
    let mut schedule = MetricsSamplingSchedule::new_due_now(now, MetricsRefreshInterval::default());

    assert!(schedule.take_due(now));
    assert_eq!(schedule.remaining(now), seconds(2));
}

#[test]
fn reschedule_waits_the_new_interval_without_immediate_sample() {
    let mut schedule = MetricsSamplingSchedule::new_due_now(seconds(0), interval(2));
    assert!(schedule.take_due(seconds(0)));

    schedule.reschedule(seconds(1), interval(60));

    assert!(!schedule.take_due(seconds(1)));
    assert_eq!(schedule.remaining(seconds(1)), seconds(60));
}

#[test]
fn delayed_wakeup_samples_once_without_a_catch_up_burst() {
    let mut schedule = MetricsSamplingSchedule::new_due_now(seconds(0), interval(2));
    assert!(schedule.take_due(seconds(0)));

    assert!(schedule.take_due(seconds(120)));
    assert!(!schedule.take_due(seconds(120)));
}

#[test]
fn backward_clock_reading_does_not_trigger_a_sample_burst() {
    let mut schedule = MetricsSamplingSchedule::new_due_now(seconds(100), interval(2));
    assert!(schedule.take_due(seconds(100)));

    assert!(!schedule.take_due(seconds(50)));
    assert!(!schedule.take_due(seconds(50)));
}
