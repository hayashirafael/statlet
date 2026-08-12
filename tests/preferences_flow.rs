use statlet::core::{AppEffect, AppEvent, Preferences, StatletCore, WarningThreshold, WindowKind};

#[test]
fn defaults_keep_disk_sampling_inactive() {
    let app = StatletCore::new();

    assert_eq!(app.state().preferences, Preferences::default());
    assert!(!app.state().preferences.mole_integration_enabled);
    assert_eq!(
        app.state().preferences.warning_threshold,
        WarningThreshold::default()
    );
}

#[test]
fn menu_actions_route_to_reusable_window_kinds() {
    let mut app = StatletCore::new();

    for _ in 0..2 {
        assert_eq!(
            app.handle(AppEvent::OpenPreferences),
            vec![AppEffect::ShowWindow(WindowKind::Preferences)]
        );
        assert_eq!(
            app.handle(AppEvent::OpenHistory),
            vec![AppEffect::ShowWindow(WindowKind::History)]
        );
    }
    assert_eq!(app.handle(AppEvent::Quit), vec![AppEffect::Quit]);
}

#[test]
fn enabling_the_integration_saves_preferences_and_starts_sampling() {
    let mut app = StatletCore::new();

    let effects = app.handle(AppEvent::SetMoleIntegrationEnabled(true));

    let expected = Preferences {
        mole_integration_enabled: true,
        ..Preferences::default()
    };
    assert_eq!(app.state().preferences, expected);
    assert_eq!(
        effects,
        vec![
            AppEffect::SavePreferences(expected.clone()),
            AppEffect::SetDiskSamplingEnabled(true),
            AppEffect::RequestNotificationAuthorization,
            AppEffect::CheckMoleCompatibility,
        ]
    );
}

#[test]
fn changing_the_threshold_saves_the_validated_preference() {
    let mut app = StatletCore::new();
    let threshold = WarningThreshold::try_from(80).unwrap();

    let effects = app.handle(AppEvent::SetWarningThreshold(threshold));

    let expected = Preferences {
        warning_threshold: threshold,
        ..Preferences::default()
    };
    assert_eq!(app.state().preferences, expected);
    assert_eq!(effects, vec![AppEffect::SavePreferences(expected.clone())]);
}

#[test]
fn startup_preferences_control_disk_sampling_without_rewriting_the_file() {
    let preferences = Preferences {
        mole_integration_enabled: true,
        warning_threshold: WarningThreshold::try_from(95).unwrap(),
        ..Preferences::default()
    };

    let (app, effects) = StatletCore::with_preferences(preferences.clone());

    assert_eq!(app.state().preferences, preferences);
    assert_eq!(
        effects,
        vec![
            AppEffect::SetDiskSamplingEnabled(true),
            AppEffect::CheckMoleCompatibility,
        ]
    );
}

#[test]
fn disabled_startup_and_enabled_to_disabled_transition_stop_disk_sampling() {
    let (mut app, startup_effects) = StatletCore::with_preferences(Preferences::default());

    assert_eq!(
        startup_effects,
        vec![AppEffect::SetDiskSamplingEnabled(false)]
    );

    app.handle(AppEvent::SetMoleIntegrationEnabled(true));
    let effects = app.handle(AppEvent::SetMoleIntegrationEnabled(false));

    assert_eq!(
        effects,
        vec![
            AppEffect::SavePreferences(Preferences::default()),
            AppEffect::SetDiskSamplingEnabled(false),
        ]
    );
}

#[test]
fn warning_threshold_accepts_only_five_point_steps_from_70_to_95() {
    for valid in [70, 75, 80, 85, 90, 95] {
        assert_eq!(WarningThreshold::try_from(valid).unwrap().get(), valid);
    }

    for invalid in [0, 69, 71, 94, 96, 100] {
        assert!(WarningThreshold::try_from(invalid).is_err());
    }
}
