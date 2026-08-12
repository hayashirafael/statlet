use std::time::Duration;

use statlet::core::{
    AppEffect, AppEvent, IndicatorPreferenceChange, MemoryPressure, Preferences,
    PreferencesSaveResult, PreferencesSaveStatus, StatletCore, SystemSnapshot, WarningThreshold,
    WindowKind,
};
use statlet::disk::DiskObservation;
use statlet::history::HistoryEventKind;
use statlet::indicator::compose_indicator;
use statlet::indicator_preferences::{IndicatorAppearance, MetricColorMode, MetricKind, SrgbColor};

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
fn repeated_preferences_requests_keep_the_same_window_kind() {
    let mut app = StatletCore::new();

    for _ in 0..2 {
        assert_eq!(
            app.handle(AppEvent::OpenPreferences),
            vec![AppEffect::ShowWindow(WindowKind::Preferences)]
        );
    }
}

#[test]
fn launch_and_reopen_address_one_retained_preferences_window() {
    let mut app = StatletCore::new();

    let mut requests = app.handle(AppEvent::ApplicationLaunched);
    assert!(app
        .handle(AppEvent::ApplicationReopened {
            has_visible_windows: true,
        })
        .is_empty());
    requests.extend(app.handle(AppEvent::ApplicationReopened {
        has_visible_windows: false,
    }));

    assert_eq!(
        requests,
        vec![
            AppEffect::ShowWindow(WindowKind::Preferences),
            AppEffect::ShowWindow(WindowKind::Preferences),
        ]
    );
}

#[test]
fn save_failure_keeps_preferences_open_for_explicit_retry() {
    let mut app = StatletCore::new();
    assert_eq!(
        app.handle(AppEvent::OpenPreferences),
        vec![AppEffect::ShowWindow(WindowKind::Preferences)]
    );

    assert!(app
        .handle(AppEvent::PreferencesSaveFinished(
            PreferencesSaveResult::Failed,
        ))
        .is_empty());

    assert_eq!(
        app.state().preferences_save_status,
        PreferencesSaveStatus::Failed
    );
    assert_eq!(
        app.handle(AppEvent::RetrySavePreferences),
        vec![AppEffect::FlushPreferences(app.state().preferences.clone())]
    );
}

#[test]
fn hidden_labels_and_fixed_colors_keep_the_complete_accessibility_label() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::MetricsSample(SystemSnapshot {
        cpu_percent: 42.0,
        ram_percent: 68.0,
        memory_pressure: MemoryPressure::Normal,
    }));
    app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetLabelsVisible(false),
    ));
    for metric in [MetricKind::Cpu, MetricKind::Ram] {
        app.handle(AppEvent::UpdateIndicator(
            IndicatorPreferenceChange::SetMetricColorMode {
                metric,
                mode: MetricColorMode::Fixed,
            },
        ));
        app.handle(AppEvent::UpdateIndicator(
            IndicatorPreferenceChange::SetMetricSharedColor {
                metric,
                color: SrgbColor::parse_hex("#777777").unwrap(),
            },
        ));
    }

    let scene = compose_indicator(
        &app.state().status,
        &app.state().preferences.indicator,
        IndicatorAppearance::Light,
    );

    assert_eq!(
        scene
            .top
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "42%"
    );
    assert_eq!(
        scene
            .bottom
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "68%"
    );
    assert_eq!(
        scene.accessibility_label,
        "CPU 42%, RAM 68%, pressão de memória normal"
    );
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
