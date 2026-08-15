use statlet::core::{AppEffect, AppEvent, IndicatorPreferenceChange, StatletCore};
use statlet::indicator::compose_indicator;
use statlet::indicator_preferences::{
    IndicatorAppearance, IndicatorLabel, IndicatorPreferences, LabelSpacing,
};

#[test]
fn labels_accept_up_to_ten_unicode_characters_and_reject_empty_or_longer_values() {
    assert_eq!(
        IndicatorLabel::new("processado").unwrap().as_str(),
        "processado"
    );
    assert!(IndicatorLabel::new("").is_err());
    assert!(IndicatorLabel::new("           ").is_err());
    assert!(IndicatorLabel::new("😀😀😀😀😀😀😀😀😀😀😀").is_err());
}

#[test]
fn label_changes_redraw_and_persist_then_render_their_independent_text_and_spacing() {
    let mut app = StatletCore::new();
    let cpu = IndicatorLabel::new("CPU uso").unwrap();
    let ram = IndicatorLabel::new("Memória").unwrap();
    let spacing = LabelSpacing::try_from(2).unwrap();

    for change in [
        IndicatorPreferenceChange::SetCpuLabel(cpu.clone()),
        IndicatorPreferenceChange::SetRamLabel(ram.clone()),
        IndicatorPreferenceChange::SetLabelSpacing(spacing),
    ] {
        let effects = app.handle(AppEvent::UpdateIndicator(change));
        assert_eq!(effects[0], AppEffect::RequestIndicatorRedraw);
        assert_eq!(
            effects[1],
            AppEffect::QueuePreferencesSave(app.state().preferences.clone())
        );
    }

    let scene = compose_indicator(
        &app.state().status,
        &app.state().preferences.indicator,
        IndicatorAppearance::Light,
    );
    assert_eq!(scene.top[0].text, "CPU uso");
    assert_eq!(scene.bottom[0].text, "Memória");
    assert_eq!(scene.top[0].trailing_spacing_level, 2);
    assert_eq!(scene.bottom[0].trailing_spacing_level, 2);
}

#[test]
fn spacing_level_ten_keeps_the_legacy_single_space_width_without_literal_label_padding() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.cpu = IndicatorLabel::new("CPU uso").unwrap();
    preferences.labels.spacing = LabelSpacing::try_from(10).unwrap();

    let scene = compose_indicator(
        &StatletCore::new().state().status,
        &preferences,
        IndicatorAppearance::Light,
    );

    assert_eq!(scene.top[0].text, "CPU uso");
    assert_eq!(scene.top[0].trailing_spacing_level, 10);
}

#[test]
fn spacing_accepts_every_decimal_level_and_rejects_values_above_one_space() {
    for level in 0..=10 {
        assert_eq!(LabelSpacing::try_from(level).unwrap().level(), level);
    }

    assert!(LabelSpacing::try_from(11).is_err());
}

#[test]
fn label_group_reset_restores_compact_c_and_r_defaults() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.cpu = IndicatorLabel::new("Processor").unwrap();
    preferences.labels.ram = IndicatorLabel::new("Memory").unwrap();
    preferences.labels.spacing = LabelSpacing::try_from(4).unwrap();

    preferences.reset(statlet::indicator_preferences::IndicatorPreferenceGroup::Labels);

    assert_eq!(preferences.labels.cpu.as_str(), "C");
    assert_eq!(preferences.labels.ram.as_str(), "R");
    assert_eq!(preferences.labels.spacing.level(), 10);
}

#[test]
fn repeated_valid_label_change_does_not_queue_a_second_save() {
    let mut app = StatletCore::new();
    let label = IndicatorLabel::new("CPU uso").unwrap();

    app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetCpuLabel(label.clone()),
    ));

    assert!(app
        .handle(AppEvent::UpdateIndicator(
            IndicatorPreferenceChange::SetCpuLabel(label),
        ))
        .is_empty());
}
