use std::time::Duration;

use statlet::core::{AppEffect, AppEvent, DiskBadge, Preferences, StatletCore};
use statlet::disk::DiskObservation;

fn observation(occupied_percent: u64, minute: u64) -> DiskObservation {
    DiskObservation::new(
        100,
        100 - occupied_percent,
        Duration::from_secs(minute * 60),
    )
    .unwrap()
}

fn enabled_core() -> StatletCore {
    StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0
}

#[test]
fn disabled_monitoring_ignores_disk_observations_and_has_no_badge() {
    let mut app = StatletCore::new();

    assert_eq!(
        app.handle(AppEvent::DiskObserved(observation(99, 0))),
        Vec::<AppEffect>::new()
    );
    assert_eq!(app.state().status.disk_badge, None);
}

#[test]
fn exact_threshold_requires_five_continuous_minutes_before_one_alert() {
    let mut app = enabled_core();

    for minute in 0..5 {
        assert_eq!(
            app.handle(AppEvent::DiskObserved(observation(90, minute))),
            Vec::<AppEffect>::new()
        );
        assert_eq!(app.state().status.disk_badge, None);
    }

    let active_observation = observation(90, 5);
    assert_eq!(
        app.handle(AppEvent::DiskObserved(active_observation)),
        vec![AppEffect::DiskPressureAlert(active_observation)]
    );
    assert_eq!(app.state().status.disk_badge, Some(DiskBadge::Warning));
    assert!(app
        .state()
        .status
        .accessibility_label
        .contains("disco acima do limite"));

    for minute in 6..10 {
        assert_eq!(
            app.handle(AppEvent::DiskObserved(observation(95, minute))),
            Vec::<AppEffect>::new()
        );
    }
}

#[test]
fn below_threshold_during_debounce_restarts_the_five_minute_window() {
    let mut app = enabled_core();

    for minute in 0..3 {
        app.handle(AppEvent::DiskObserved(observation(91, minute)));
    }
    app.handle(AppEvent::DiskObserved(observation(89, 3)));

    for minute in 4..9 {
        assert!(app
            .handle(AppEvent::DiskObserved(observation(91, minute)))
            .is_empty());
    }
    assert_eq!(app.state().status.disk_badge, None);
    assert!(!app
        .state()
        .status
        .accessibility_label
        .contains("disco acima do limite"));

    let restarted_episode = observation(91, 9);
    assert_eq!(
        app.handle(AppEvent::DiskObserved(restarted_episode)),
        vec![AppEffect::DiskPressureAlert(restarted_episode)]
    );
}

#[test]
fn below_threshold_after_alert_clears_the_badge_and_rearms() {
    let mut app = enabled_core();
    for minute in 0..=5 {
        app.handle(AppEvent::DiskObserved(observation(90, minute)));
    }

    assert!(app
        .handle(AppEvent::DiskObserved(observation(89, 6)))
        .is_empty());
    assert_eq!(app.state().status.disk_badge, None);

    for minute in 7..12 {
        assert!(app
            .handle(AppEvent::DiskObserved(observation(90, minute)))
            .is_empty());
    }
    let second_episode = observation(90, 12);
    assert_eq!(
        app.handle(AppEvent::DiskObserved(second_episode)),
        vec![AppEffect::DiskPressureAlert(second_episode)]
    );
}

#[test]
fn disabling_clears_an_active_episode_and_ignores_future_samples() {
    let mut app = enabled_core();
    for minute in 0..=5 {
        app.handle(AppEvent::DiskObserved(observation(90, minute)));
    }
    assert_eq!(app.state().status.disk_badge, Some(DiskBadge::Warning));

    app.handle(AppEvent::SetMoleIntegrationEnabled(false));

    assert_eq!(app.state().status.disk_badge, None);
    assert!(app
        .handle(AppEvent::DiskObserved(observation(99, 20)))
        .is_empty());
}

#[test]
fn an_unobserved_sleep_gap_restarts_debounce_instead_of_counting_as_pressure() {
    let mut app = enabled_core();

    app.handle(AppEvent::DiskObserved(observation(90, 0)));
    app.handle(AppEvent::DiskObserved(observation(90, 1)));
    assert!(app
        .handle(AppEvent::DiskObserved(observation(90, 5)))
        .is_empty());
    assert_eq!(app.state().status.disk_badge, None);

    for minute in 6..10 {
        assert!(app
            .handle(AppEvent::DiskObserved(observation(90, minute)))
            .is_empty());
    }
    let after_five_observed_minutes = observation(90, 10);
    assert_eq!(
        app.handle(AppEvent::DiskObserved(after_five_observed_minutes)),
        vec![AppEffect::DiskPressureAlert(after_five_observed_minutes)]
    );
}
