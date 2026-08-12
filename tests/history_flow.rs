use std::time::Duration;

use statlet::core::{AppEffect, AppEvent, Preferences, StatletCore};
use statlet::disk::DiskObservation;
use statlet::history::HistoryEventKind;
use statlet::mole::{MoleStatus, MoleVersion};

fn observation(occupied: u64, minute: u64) -> DiskObservation {
    DiskObservation::new(100, 100 - occupied, Duration::from_secs(minute * 60)).unwrap()
}

fn enabled_app() -> StatletCore {
    StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0
}

#[test]
fn active_disk_episode_records_start_and_recovery_once() {
    let mut app = enabled_app();
    for minute in 0..5 {
        assert!(app
            .handle(AppEvent::DiskObserved(observation(90, minute)))
            .is_empty());
    }
    let started = observation(90, 5);

    assert_eq!(
        app.handle(AppEvent::DiskObserved(started)),
        vec![
            AppEffect::RecordHistory(HistoryEventKind::DiskPressureStarted),
            AppEffect::DiskPressureAlert(started),
        ]
    );
    assert!(app
        .handle(AppEvent::DiskObserved(observation(91, 6)))
        .is_empty());
    assert_eq!(
        app.handle(AppEvent::DiskObserved(observation(89, 7))),
        vec![AppEffect::RecordHistory(
            HistoryEventKind::DiskPressureRecovered
        )]
    );
    assert!(app
        .handle(AppEvent::DiskObserved(observation(88, 8)))
        .is_empty());
}

#[test]
fn integration_blocks_are_recorded_once_until_mole_recovers() {
    let mut app = enabled_app();

    assert_eq!(
        app.handle(AppEvent::MoleStatusObserved(MoleStatus::Missing)),
        vec![
            AppEffect::RedrawIndicator,
            AppEffect::RecordHistory(HistoryEventKind::MoleMissing),
        ]
    );
    assert!(app
        .handle(AppEvent::MoleStatusObserved(MoleStatus::Missing))
        .is_empty());
    assert_eq!(
        app.handle(AppEvent::MoleStatusObserved(MoleStatus::Compatible(
            MoleVersion::new(1, 49, 2)
        ))),
        vec![AppEffect::RedrawIndicator]
    );
    assert_eq!(
        app.handle(AppEvent::MoleStatusObserved(MoleStatus::Incompatible(
            MoleVersion::new(2, 0, 0)
        ))),
        vec![
            AppEffect::RedrawIndicator,
            AppEffect::RecordHistory(HistoryEventKind::MoleIncompatible),
        ]
    );
}

#[test]
fn monitoring_failure_is_an_episode_and_success_rearms_it() {
    let mut app = enabled_app();

    assert_eq!(
        app.handle(AppEvent::DiskMonitoringFailed),
        vec![AppEffect::RecordHistory(HistoryEventKind::MonitoringFailed)]
    );
    assert!(app.handle(AppEvent::DiskMonitoringFailed).is_empty());
    assert!(app
        .handle(AppEvent::DiskObserved(observation(50, 1)))
        .is_empty());
    assert_eq!(
        app.handle(AppEvent::DiskMonitoringFailed),
        vec![AppEffect::RecordHistory(HistoryEventKind::MonitoringFailed)]
    );
}

#[test]
fn external_terminal_flow_never_claims_a_statlet_cleanup() {
    let mut app = enabled_app();
    app.handle(AppEvent::MoleStatusObserved(MoleStatus::Compatible(
        MoleVersion::new(1, 49, 2),
    )));

    assert_eq!(
        app.handle(AppEvent::OpenMoleInTerminal),
        vec![AppEffect::LaunchMoleInTerminal]
    );
}

#[test]
fn only_the_confirmed_clear_event_requests_history_deletion() {
    let mut app = enabled_app();

    assert_eq!(
        app.handle(AppEvent::ClearHistoryConfirmed),
        vec![AppEffect::ClearHistory]
    );
}
