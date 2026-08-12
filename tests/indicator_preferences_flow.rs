use statlet::core::{
    AppEffect, AppEvent, IndicatorPreferenceChange, Preferences, StatletCore, WarningThreshold,
};
use statlet::indicator_preferences::{
    FontFamilyPreference, FontSize, FontWeight, IndicatorAppearance, IndicatorPreferenceGroup,
    IndicatorPreferences, LabelColorMode, MetricColorMode, MetricKind, MetricsRefreshInterval,
    SrgbColor,
};

fn customized_app(mole_enabled: bool) -> StatletCore {
    let mut indicator = IndicatorPreferences::default();
    indicator.typography.size = FontSize::try_from(14).unwrap();
    let preferences = Preferences {
        mole_integration_enabled: mole_enabled,
        warning_threshold: WarningThreshold::try_from(80).unwrap(),
        indicator,
    };
    StatletCore::with_preferences(preferences).0
}

fn assert_redraw_then_save(effects: Vec<AppEffect>, app: &StatletCore) {
    assert_eq!(
        effects,
        vec![
            AppEffect::RedrawIndicator,
            AppEffect::SavePreferences(app.state().preferences.clone()),
        ]
    );
}

#[test]
fn visual_change_redraws_then_saves_the_complete_document() {
    let mut app = StatletCore::new();
    let color = SrgbColor::parse_hex("#AF52DE").unwrap();

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricSharedColor {
            metric: MetricKind::Cpu,
            color,
        },
    ));

    assert_eq!(effects[0], AppEffect::RedrawIndicator);
    assert_eq!(
        effects[1],
        AppEffect::SavePreferences(app.state().preferences.clone())
    );
}

#[test]
fn interval_change_reschedules_without_collecting() {
    let mut app = StatletCore::new();
    let interval = MetricsRefreshInterval::try_from(17).unwrap();

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetRefreshInterval(interval),
    ));

    assert_eq!(
        effects,
        vec![
            AppEffect::SetMetricsSamplingInterval(interval),
            AppEffect::RedrawIndicator,
            AppEffect::SavePreferences(app.state().preferences.clone()),
        ]
    );
}

#[test]
fn repeated_visual_change_emits_no_effect() {
    let mut app = StatletCore::new();
    let change = IndicatorPreferenceChange::SetMetricSharedColor {
        metric: MetricKind::Ram,
        color: SrgbColor::parse_hex("#AF52DE").unwrap(),
    };

    app.handle(AppEvent::UpdateIndicator(change.clone()));

    assert!(app.handle(AppEvent::UpdateIndicator(change)).is_empty());
}

#[test]
fn unchanged_appearance_color_does_not_create_hidden_variants() {
    let mut app = StatletCore::new();
    let default_cpu = IndicatorPreferences::default().cpu_color;

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricAppearanceColor {
            metric: MetricKind::Cpu,
            appearance: IndicatorAppearance::Light,
            color: default_cpu.fixed.shared,
        },
    ));

    assert!(effects.is_empty());
    assert_eq!(app.state().preferences.indicator.cpu_color, default_cpu);
}

#[test]
fn unchanged_interval_does_not_reschedule() {
    let mut app = StatletCore::new();
    let interval = MetricsRefreshInterval::try_from(17).unwrap();

    app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetRefreshInterval(interval),
    ));

    assert!(app
        .handle(AppEvent::UpdateIndicator(
            IndicatorPreferenceChange::SetRefreshInterval(interval),
        ))
        .is_empty());
}

#[test]
fn global_reset_keeps_disk_preferences_and_undo_replaces_later_indicator_edits() {
    let mut app = customized_app(true);
    let before = app.state().preferences.indicator.clone();

    app.handle(AppEvent::ResetIndicatorConfirmed);

    assert_eq!(
        app.state().preferences.indicator,
        IndicatorPreferences::default()
    );
    assert!(app.state().can_undo_indicator_reset);
    assert!(app.state().preferences.mole_integration_enabled);
    assert_eq!(app.state().preferences.warning_threshold.get(), 80);

    app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetFontSize(FontSize::try_from(14).unwrap()),
    ));
    app.handle(AppEvent::UndoIndicatorReset);

    assert_eq!(app.state().preferences.indicator, before);
    assert!(app.state().preferences.mole_integration_enabled);
    assert_eq!(app.state().preferences.warning_threshold.get(), 80);
    assert!(!app.state().can_undo_indicator_reset);
}

#[test]
fn closing_preferences_discards_only_the_transient_undo_snapshot() {
    let mut app = customized_app(false);

    app.handle(AppEvent::ResetIndicatorConfirmed);
    app.handle(AppEvent::PreferencesWindowClosed);

    assert!(app.handle(AppEvent::UndoIndicatorReset).is_empty());
    assert_eq!(
        app.state().preferences.indicator,
        IndicatorPreferences::default()
    );
}

#[test]
fn group_reset_changes_only_that_indicator_group() {
    let mut indicator = IndicatorPreferences::default();
    indicator.typography.size = FontSize::try_from(14).unwrap();
    indicator.refresh_interval = MetricsRefreshInterval::try_from(17).unwrap();
    let preferences = Preferences {
        mole_integration_enabled: true,
        indicator,
        ..Preferences::default()
    };
    let mut app = StatletCore::with_preferences(preferences).0;

    let effects = app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Typography,
    ));

    assert_eq!(
        app.state().preferences.indicator.typography,
        IndicatorPreferences::default().typography
    );
    assert_eq!(
        app.state().preferences.indicator.refresh_interval.seconds(),
        17
    );
    assert!(app.state().preferences.mole_integration_enabled);
    assert_eq!(
        effects,
        vec![
            AppEffect::RedrawIndicator,
            AppEffect::SavePreferences(app.state().preferences.clone()),
        ]
    );
}

#[test]
fn refresh_interval_group_reset_reschedules_to_the_default() {
    let indicator = IndicatorPreferences {
        refresh_interval: MetricsRefreshInterval::try_from(17).unwrap(),
        ..IndicatorPreferences::default()
    };
    let preferences = Preferences {
        indicator,
        ..Preferences::default()
    };
    let mut app = StatletCore::with_preferences(preferences).0;
    let default_interval = MetricsRefreshInterval::try_from(2).unwrap();

    let effects = app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::RefreshInterval,
    ));

    assert_eq!(
        effects,
        vec![
            AppEffect::SetMetricsSamplingInterval(default_interval),
            AppEffect::RedrawIndicator,
            AppEffect::SavePreferences(app.state().preferences.clone()),
        ]
    );
}

#[test]
fn global_reset_and_undo_reschedule_only_when_the_interval_changes() {
    let customized_interval = MetricsRefreshInterval::try_from(17).unwrap();
    let default_interval = MetricsRefreshInterval::try_from(2).unwrap();
    let indicator = IndicatorPreferences {
        refresh_interval: customized_interval,
        ..IndicatorPreferences::default()
    };
    let preferences = Preferences {
        indicator,
        ..Preferences::default()
    };
    let mut app = StatletCore::with_preferences(preferences).0;

    let reset_effects = app.handle(AppEvent::ResetIndicatorConfirmed);

    assert_eq!(
        reset_effects,
        vec![
            AppEffect::SetMetricsSamplingInterval(default_interval),
            AppEffect::RedrawIndicator,
            AppEffect::SavePreferences(app.state().preferences.clone()),
        ]
    );

    let undo_effects = app.handle(AppEvent::UndoIndicatorReset);

    assert_eq!(
        undo_effects,
        vec![
            AppEffect::SetMetricsSamplingInterval(customized_interval),
            AppEffect::RedrawIndicator,
            AppEffect::SavePreferences(app.state().preferences.clone()),
        ]
    );
}

#[test]
fn a_second_global_reset_replaces_the_previous_undo_snapshot() {
    let mut app = customized_app(false);
    app.handle(AppEvent::ResetIndicatorConfirmed);
    app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetFontSize(FontSize::try_from(13).unwrap()),
    ));

    app.handle(AppEvent::ResetIndicatorConfirmed);
    app.handle(AppEvent::UndoIndicatorReset);

    assert_eq!(
        app.state().preferences.indicator.typography.size.points(),
        13
    );
    assert!(!app.state().can_undo_indicator_reset);
}

#[test]
fn metric_visual_changes_update_only_the_requested_metric() {
    let mut app = StatletCore::new();

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricColorMode {
            metric: MetricKind::Cpu,
            mode: MetricColorMode::Fixed,
        },
    ));
    assert_eq!(
        app.state().preferences.indicator.cpu_color.mode,
        MetricColorMode::Fixed
    );
    assert_eq!(
        app.state().preferences.indicator.ram_color.mode,
        MetricColorMode::Dynamic
    );
    assert_redraw_then_save(effects, &app);

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricVariantsEnabled {
            metric: MetricKind::Cpu,
            enabled: true,
        },
    ));
    assert!(
        app.state()
            .preferences
            .indicator
            .cpu_color
            .fixed
            .use_appearance_variants
    );
    assert_redraw_then_save(effects, &app);

    let color = SrgbColor::parse_hex("#BF5AF2").unwrap();
    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricAppearanceColor {
            metric: MetricKind::Cpu,
            appearance: IndicatorAppearance::Dark,
            color,
        },
    ));
    assert_eq!(
        app.state()
            .preferences
            .indicator
            .cpu_color
            .fixed
            .variants
            .unwrap()
            .dark,
        color
    );
    assert_redraw_then_save(effects, &app);
}

#[test]
fn label_visual_changes_update_their_retained_preferences() {
    let mut app = StatletCore::new();

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetLabelsVisible(false),
    ));
    assert!(!app.state().preferences.indicator.labels.visible);
    assert_redraw_then_save(effects, &app);

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetLabelColorMode(LabelColorMode::Fixed),
    ));
    assert_eq!(
        app.state().preferences.indicator.labels.color_mode,
        LabelColorMode::Fixed
    );
    assert_redraw_then_save(effects, &app);

    let shared = SrgbColor::parse_hex("#FF9F0A").unwrap();
    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetLabelSharedColor(shared),
    ));
    assert_eq!(
        app.state().preferences.indicator.labels.fixed.shared,
        shared
    );
    assert_redraw_then_save(effects, &app);

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetLabelVariantsEnabled(true),
    ));
    assert!(
        app.state()
            .preferences
            .indicator
            .labels
            .fixed
            .use_appearance_variants
    );
    assert_redraw_then_save(effects, &app);

    let light = SrgbColor::parse_hex("#FFD60A").unwrap();
    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetLabelAppearanceColor {
            appearance: IndicatorAppearance::Light,
            color: light,
        },
    ));
    assert_eq!(
        app.state()
            .preferences
            .indicator
            .labels
            .fixed
            .variants
            .unwrap()
            .light,
        light
    );
    assert_redraw_then_save(effects, &app);
}

#[test]
fn typography_changes_redraw_and_save_the_complete_document() {
    let mut app = StatletCore::new();
    let family = FontFamilyPreference::named("Avenir Next").unwrap();

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetFontFamily(family.clone()),
    ));
    assert_eq!(app.state().preferences.indicator.typography.family, family);
    assert_redraw_then_save(effects, &app);

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetFontWeight(FontWeight::Bold),
    ));
    assert_eq!(
        app.state().preferences.indicator.typography.weight,
        FontWeight::Bold
    );
    assert_redraw_then_save(effects, &app);
}
