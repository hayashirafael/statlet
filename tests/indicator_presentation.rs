use statlet::core::{DiskBadge, MetricContent, MetricSeverity, StatusContent};
use statlet::indicator::{compose_indicator, SegmentColor, SemanticColor};
use statlet::indicator_preferences::{
    IndicatorAppearance, IndicatorPreferences, LabelColorMode, MetricColorMode, SrgbColor,
};

fn status() -> StatusContent {
    StatusContent {
        cpu: MetricContent {
            label: "C",
            percent: 42,
            severity: MetricSeverity::Warning,
        },
        ram: MetricContent {
            label: "R",
            percent: 68,
            severity: MetricSeverity::Good,
        },
        disk_badge: None,
        accessibility_label: "CPU 42%, RAM 68%, pressão de memória normal".into(),
    }
}

fn critical_status() -> StatusContent {
    let mut status = status();
    status.cpu.severity = MetricSeverity::Critical;
    status
}

#[test]
fn dynamic_metrics_and_neutral_labels_create_unpadded_runs() {
    let scene = compose_indicator(
        &status(),
        &IndicatorPreferences::default(),
        IndicatorAppearance::Light,
    );

    assert_eq!(scene.top[0].text, "C ");
    assert_eq!(
        scene.top[0].color,
        SegmentColor::Semantic(SemanticColor::Neutral)
    );
    assert_eq!(scene.top[1].text, "42%");
    assert_eq!(
        scene.top[1].color,
        SegmentColor::Semantic(SemanticColor::Warning)
    );
    assert_eq!(scene.bottom[0].text, "R ");
    assert_eq!(
        scene.bottom[0].color,
        SegmentColor::Semantic(SemanticColor::Neutral)
    );
    assert_eq!(scene.bottom[1].text, "68%");
    assert_eq!(
        scene.bottom[1].color,
        SegmentColor::Semantic(SemanticColor::Good)
    );
    assert!(scene.disk_badge.is_none());
}

#[test]
fn critical_dynamic_metric_uses_the_critical_semantic_color() {
    let scene = compose_indicator(
        &critical_status(),
        &IndicatorPreferences::default(),
        IndicatorAppearance::Light,
    );

    assert_eq!(
        scene.top.last().unwrap().color,
        SegmentColor::Semantic(SemanticColor::Critical)
    );
}

#[test]
fn hidden_labels_remove_text_but_not_accessibility() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.visible = false;

    let scene = compose_indicator(&status(), &preferences, IndicatorAppearance::Light);

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
    assert!(scene.accessibility_label.starts_with("CPU 42%, RAM 68%"));
}

#[test]
fn fixed_cpu_and_dark_variant_ignore_severity() {
    let mut preferences = IndicatorPreferences::default();
    preferences.cpu_color.mode = MetricColorMode::Fixed;
    preferences.cpu_color.fixed.set_variants_enabled(true);
    preferences.cpu_color.fixed.variants.as_mut().unwrap().dark =
        SrgbColor::parse_hex("#AF52DE").unwrap();
    preferences.ram_color.mode = MetricColorMode::Fixed;
    preferences.ram_color.fixed.shared = SrgbColor::parse_hex("#00C7BE").unwrap();

    let scene = compose_indicator(&critical_status(), &preferences, IndicatorAppearance::Dark);

    assert_eq!(
        scene.top.last().unwrap().color,
        SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap())
    );
    assert_eq!(
        scene.bottom.last().unwrap().color,
        SegmentColor::Srgb(SrgbColor::parse_hex("#00C7BE").unwrap())
    );
}

#[test]
fn matching_labels_copy_the_resolved_color_of_each_metric() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.color_mode = LabelColorMode::MatchMetric;
    preferences.cpu_color.mode = MetricColorMode::Fixed;
    preferences.cpu_color.fixed.shared = SrgbColor::parse_hex("#123456").unwrap();

    let scene = compose_indicator(&status(), &preferences, IndicatorAppearance::Light);

    assert_eq!(
        scene.top[0].color,
        SegmentColor::Srgb(SrgbColor::parse_hex("#123456").unwrap())
    );
    assert_eq!(scene.top[0].color, scene.top[1].color);
    assert_eq!(
        scene.bottom[0].color,
        SegmentColor::Semantic(SemanticColor::Good)
    );
    assert_eq!(scene.bottom[0].color, scene.bottom[1].color);
}

#[test]
fn fixed_labels_use_their_own_appearance_variant() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.color_mode = LabelColorMode::Fixed;
    preferences.labels.fixed.set_variants_enabled(true);
    preferences.labels.fixed.variants.as_mut().unwrap().dark =
        SrgbColor::parse_hex("#FF9F0A").unwrap();

    let scene = compose_indicator(&status(), &preferences, IndicatorAppearance::Dark);
    let expected = SegmentColor::Srgb(SrgbColor::parse_hex("#FF9F0A").unwrap());

    assert_eq!(scene.top[0].color, expected);
    assert_eq!(scene.bottom[0].color, expected);
    assert_ne!(scene.top[0].color, scene.top[1].color);
}

#[test]
fn disk_badges_keep_their_symbols_and_semantic_colors() {
    let mut warning = status();
    warning.disk_badge = Some(DiskBadge::Warning);
    let mut error = status();
    error.disk_badge = Some(DiskBadge::Error);

    let warning_scene = compose_indicator(
        &warning,
        &IndicatorPreferences::default(),
        IndicatorAppearance::Light,
    );
    let error_scene = compose_indicator(
        &error,
        &IndicatorPreferences::default(),
        IndicatorAppearance::Light,
    );

    let warning_badge = warning_scene.disk_badge.unwrap();
    let error_badge = error_scene.disk_badge.unwrap();
    assert_eq!(warning_badge.text, " !");
    assert_eq!(
        warning_badge.color,
        SegmentColor::Semantic(SemanticColor::DiskWarning)
    );
    assert_eq!(
        warning_scene.top.last().unwrap().text,
        "42%",
        "the badge remains separate from the top metric"
    );
    assert_eq!(error_badge.text, " ×");
    assert_eq!(
        error_badge.color,
        SegmentColor::Semantic(SemanticColor::DiskError)
    );
}
