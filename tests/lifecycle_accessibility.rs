use std::time::Duration;

use statlet::core::{AppEffect, AppEvent, Preferences, StatletCore, WarningThreshold, WindowKind};
use statlet::disk::DiskObservation;
use statlet::history::HistoryEventKind;

fn enabled_core() -> StatletCore {
    StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        warning_threshold: WarningThreshold::try_from(90).unwrap(),
        ..Preferences::default()
    })
    .0
}

fn observation(occupied_percent: u64, minute: u64) -> DiskObservation {
    DiskObservation::new(
        100,
        100 - occupied_percent,
        Duration::from_secs(minute * 60),
    )
    .unwrap()
}

#[test]
fn a_direct_launch_exposes_preferences_even_without_a_status_item() {
    let mut app = StatletCore::new();

    assert_eq!(
        app.handle(AppEvent::ApplicationLaunched),
        vec![AppEffect::ShowWindow(WindowKind::Preferences)]
    );
}

#[test]
fn reopening_without_visible_windows_exposes_preferences() {
    let mut app = StatletCore::new();

    assert_eq!(
        app.handle(AppEvent::ApplicationReopened {
            has_visible_windows: false,
        }),
        vec![AppEffect::ShowWindow(WindowKind::Preferences)]
    );
    assert!(app
        .handle(AppEvent::ApplicationReopened {
            has_visible_windows: true,
        })
        .is_empty());
}

#[test]
fn an_active_episode_survives_sleep_without_duplicate_alerts() {
    let mut app = enabled_core();
    for minute in 0..5 {
        assert!(app
            .handle(AppEvent::DiskObserved(observation(90, minute)))
            .is_empty());
    }
    let first_alert = observation(90, 5);
    assert_eq!(
        app.handle(AppEvent::DiskObserved(first_alert)),
        vec![
            AppEffect::RecordHistory(HistoryEventKind::DiskPressureStarted),
            AppEffect::DiskPressureAlert(first_alert),
        ]
    );

    assert!(app
        .handle(AppEvent::DiskObserved(observation(92, 125)))
        .is_empty());
    assert!(app
        .handle(AppEvent::DiskObserved(observation(92, 126)))
        .is_empty());

    assert_eq!(
        app.handle(AppEvent::DiskObserved(observation(89, 127))),
        vec![AppEffect::RecordHistory(
            HistoryEventKind::DiskPressureRecovered
        )]
    );
    assert!(app
        .handle(AppEvent::DiskObserved(observation(88, 128)))
        .is_empty());
}
