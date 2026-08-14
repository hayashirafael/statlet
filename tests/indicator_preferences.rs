use statlet::indicator_preferences::{
    FontFamilyPreference, FontSize, IndicatorAppearance, IndicatorPreferenceGroup,
    IndicatorPreferences, LabelColorMode, MetricIdentifierMode, MetricsRefreshInterval, SrgbColor,
};

#[test]
fn hex_accepts_hash_or_bare_rgb_and_normalizes_uppercase() {
    assert_eq!(SrgbColor::parse_hex("#0a84ff").unwrap().to_hex(), "#0A84FF");
    assert_eq!(SrgbColor::parse_hex("34c759").unwrap().to_hex(), "#34C759");
}

#[test]
fn hex_rejects_alpha_short_and_non_hex_values() {
    for value in ["#0A84FFFF", "#FFF", "0A84F", "#GG84FF"] {
        assert!(SrgbColor::parse_hex(value).is_err(), "accepted {value}");
    }
}

#[test]
fn hex_rejects_non_ascii_input_without_panicking() {
    let result = std::panic::catch_unwind(|| SrgbColor::parse_hex("AéAAA"));
    assert!(result.is_ok(), "parser panicked for non-ASCII input");
    assert!(result.unwrap().is_err());
}

#[test]
fn bounded_values_accept_only_the_approved_ranges() {
    assert!(FontSize::try_from(8).is_err());
    assert_eq!(FontSize::try_from(9).unwrap().points(), 9);
    assert_eq!(FontSize::try_from(14).unwrap().points(), 14);
    assert!(FontSize::try_from(15).is_err());
    assert!(MetricsRefreshInterval::try_from(0).is_err());
    assert_eq!(MetricsRefreshInterval::try_from(60).unwrap().seconds(), 60);
    assert!(MetricsRefreshInterval::try_from(61).is_err());
}

#[test]
fn refresh_interval_exposes_its_canonical_limits() {
    assert_eq!(MetricsRefreshInterval::MIN_SECONDS, 1);
    assert_eq!(MetricsRefreshInterval::MAX_SECONDS, 60);
}

#[test]
fn defaults_exactly_match_the_v1_indicator() {
    let value = IndicatorPreferences::default();
    assert!(value.labels.visible);
    assert_eq!(value.typography.size.points(), 12);
    assert_eq!(value.refresh_interval.seconds(), 2);
    assert!(value.cpu_color.is_dynamic());
    assert!(value.ram_color.is_dynamic());
}

#[test]
fn disabling_variants_preserves_them_for_reenable_but_group_reset_removes_them() {
    let mut value = IndicatorPreferences::default();
    value.cpu_color.fixed.set_variants_enabled(true);
    let remembered = value.cpu_color.fixed.variants.unwrap();
    value.cpu_color.fixed.set_variants_enabled(false);
    assert_eq!(value.cpu_color.fixed.variants, Some(remembered));
    value.cpu_color.fixed.set_variants_enabled(true);
    assert_eq!(value.cpu_color.fixed.variants, Some(remembered));
    value.reset(IndicatorPreferenceGroup::CpuAndRam);
    assert_eq!(value.cpu_color, IndicatorPreferences::default().cpu_color);
}

#[test]
fn enabled_variants_override_the_shared_color_for_each_appearance() {
    let mut fixed = IndicatorPreferences::default().cpu_color.fixed;
    let shared = fixed.shared;
    assert_eq!(fixed.color_for(IndicatorAppearance::Dark), shared);

    fixed.set_variants_enabled(true);
    let custom_dark = SrgbColor::parse_hex("#AF52DE").unwrap();
    fixed.variants.as_mut().unwrap().dark = custom_dark;

    assert_eq!(fixed.color_for(IndicatorAppearance::Light), shared);
    assert_eq!(fixed.color_for(IndicatorAppearance::Dark), custom_dark);
}

#[test]
fn named_font_trims_a_name_but_rejects_blank_input() {
    assert_eq!(
        FontFamilyPreference::named("  Avenir Next  ").unwrap(),
        FontFamilyPreference::Named("Avenir Next".to_owned())
    );
    assert!(FontFamilyPreference::named(" \t ").is_err());
}

#[test]
fn reset_leaves_other_indicator_groups_unchanged() {
    let mut value = IndicatorPreferences::default();
    value.labels.visible = false;
    value.labels.color_mode = LabelColorMode::Fixed;
    value.refresh_interval = MetricsRefreshInterval::try_from(7).unwrap();

    value.reset(IndicatorPreferenceGroup::CpuAndRam);

    assert!(!value.labels.visible);
    assert_eq!(value.labels.color_mode, LabelColorMode::Fixed);
    assert_eq!(value.refresh_interval.seconds(), 7);
}

#[test]
fn color_and_identifier_resets_have_separate_scopes() {
    let mut value = IndicatorPreferences::default();
    value.cpu_color.fixed.set_variants_enabled(true);
    value.cpu_color.mode = statlet::indicator_preferences::MetricColorMode::Fixed;
    value.identifiers.cpu.mode = MetricIdentifierMode::SystemSymbol;
    let identifiers = value.identifiers.clone();

    value.reset(IndicatorPreferenceGroup::CpuAndRam);

    assert_eq!(value.cpu_color, IndicatorPreferences::default().cpu_color);
    assert_eq!(value.identifiers, identifiers);

    value.reset(IndicatorPreferenceGroup::Identifiers);

    assert_eq!(
        value.identifiers,
        IndicatorPreferences::default().identifiers
    );
    assert_eq!(value.cpu_color, IndicatorPreferences::default().cpu_color);
}
