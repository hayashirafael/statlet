use std::time::Duration;

use statlet::core::{AppEffect, AppEvent, DiskBadge, Preferences, StatletCore, WindowKind};
use statlet::disk::DiskObservation;
use statlet::history::HistoryEventKind;
use statlet::mole::{MoleStatus, MoleVersion};

fn observation(occupied: u64, minute: u64) -> DiskObservation {
    DiskObservation::new(100, 100 - occupied, Duration::from_secs(minute * 60)).unwrap()
}

#[test]
fn enabling_requests_notification_permission_and_checks_mole_in_context() {
    let mut app = StatletCore::new();

    let effects = app.handle(AppEvent::SetMoleIntegrationEnabled(true));

    assert_eq!(
        effects,
        vec![
            AppEffect::QueuePreferencesSave(Preferences {
                mole_integration_enabled: true,
                ..Preferences::default()
            }),
            AppEffect::SetDiskSamplingEnabled(true),
            AppEffect::RequestNotificationAuthorization,
            AppEffect::CheckMoleCompatibility,
        ]
    );
}

#[test]
fn restored_opt_in_checks_mole_without_reprompting_for_notifications() {
    let (_, effects) = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    });

    assert_eq!(
        effects,
        vec![
            AppEffect::SetMenuBarVisible(true),
            AppEffect::SetDiskSamplingEnabled(true),
            AppEffect::CheckMoleCompatibility,
        ]
    );
}

#[test]
fn notification_and_menu_route_to_the_same_free_space_window() {
    let mut app = StatletCore::new();

    assert_eq!(
        app.handle(AppEvent::ReviewSpace),
        vec![AppEffect::ShowWindow(WindowKind::FreeSpace)]
    );
    assert_eq!(
        app.handle(AppEvent::NotificationActivated),
        vec![AppEffect::ShowWindow(WindowKind::FreeSpace)]
    );
}

#[test]
fn opening_free_space_rechecks_mole_without_blocking_the_window() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;
    app.handle(AppEvent::MoleStatusObserved(MoleStatus::Missing));

    assert_eq!(
        app.handle(AppEvent::ReviewSpace),
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::CheckMoleCompatibility,
            AppEffect::ShowWindow(WindowKind::FreeSpace),
        ]
    );
    assert_eq!(app.state().mole_status, MoleStatus::Unknown);
    assert!(app.handle(AppEvent::OpenMoleInTerminal).is_empty());
}

#[test]
fn latest_startup_volume_observation_is_available_to_the_read_only_window() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;
    let latest = observation(82, 0);

    app.handle(AppEvent::DiskObserved(latest));

    assert_eq!(app.state().latest_disk_observation, Some(latest));
}

#[test]
fn missing_or_incompatible_mole_uses_a_red_symbolic_badge() {
    for status in [
        MoleStatus::Missing,
        MoleStatus::Incompatible(MoleVersion::new(2, 0, 0)),
    ] {
        let mut app = StatletCore::with_preferences(Preferences {
            mole_integration_enabled: true,
            ..Preferences::default()
        })
        .0;

        app.handle(AppEvent::MoleStatusObserved(status));

        assert_eq!(app.state().status.disk_badge, Some(DiskBadge::Error));
        assert!(app
            .state()
            .status
            .accessibility_label
            .contains("Mole indisponível"));
    }
}

#[test]
fn asynchronous_mole_error_redraws_once_before_recording_history() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;

    let effects = app.handle(AppEvent::MoleStatusObserved(MoleStatus::Missing));

    assert_eq!(app.state().status.disk_badge, Some(DiskBadge::Error));
    assert_eq!(
        effects,
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::RecordHistory(HistoryEventKind::MoleMissing),
        ]
    );
}

#[test]
fn asynchronous_compatible_mole_status_redraws_once_when_it_clears_the_error_badge() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;
    app.handle(AppEvent::MoleStatusObserved(MoleStatus::Missing));
    assert_eq!(app.state().status.disk_badge, Some(DiskBadge::Error));

    let effects = app.handle(AppEvent::MoleStatusObserved(MoleStatus::Compatible(
        MoleVersion::new(1, 49, 2),
    )));

    assert_eq!(app.state().status.disk_badge, None);
    assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
}

#[test]
fn disabling_mole_with_an_active_badge_redraws_before_saving_and_stopping_sampling() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;
    app.handle(AppEvent::MoleStatusObserved(MoleStatus::Missing));
    assert_eq!(app.state().status.disk_badge, Some(DiskBadge::Error));

    let effects = app.handle(AppEvent::SetMoleIntegrationEnabled(false));

    assert_eq!(app.state().status.disk_badge, None);
    assert_eq!(
        effects,
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::QueuePreferencesSave(Preferences::default()),
            AppEffect::SetDiskSamplingEnabled(false),
        ]
    );
}

#[test]
fn explicit_terminal_action_is_the_only_effect_that_can_launch_mole() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;
    app.handle(AppEvent::MoleStatusObserved(MoleStatus::Compatible(
        MoleVersion::new(1, 49, 2),
    )));

    assert_eq!(
        app.handle(AppEvent::OpenMoleInTerminal),
        vec![AppEffect::LaunchMoleInTerminal]
    );
}

#[test]
fn threshold_crossing_and_opening_the_window_never_launch_mole() {
    let mut app = StatletCore::with_preferences(Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    })
    .0;
    for minute in 0..5 {
        assert!(app
            .handle(AppEvent::DiskObserved(observation(90, minute)))
            .is_empty());
    }

    let alert = observation(90, 5);
    assert_eq!(
        app.handle(AppEvent::DiskObserved(alert)),
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::RecordHistory(HistoryEventKind::DiskPressureStarted),
            AppEffect::DiskPressureAlert(alert),
        ]
    );
    assert_eq!(
        app.handle(AppEvent::ReviewSpace),
        vec![
            AppEffect::CheckMoleCompatibility,
            AppEffect::ShowWindow(WindowKind::FreeSpace),
        ]
    );
}
