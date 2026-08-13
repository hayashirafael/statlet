use std::fs;

use statlet::core::{AppEffect, AppEvent, IndicatorPreferenceChange, Preferences, StatletCore};
use statlet::core::{MetricContent, MetricSeverity, StatusContent};
use statlet::indicator::{
    compose_indicator, preview_accessibility_summary, MetricIdentifierVisual, SegmentColor,
    SemanticColor,
};
use statlet::indicator_preferences::{
    IndicatorLabel, IndicatorPreferences, MetricIdentifierMode, MetricKind, PngIconMetadata,
    SystemSymbolName,
};
use statlet::preferences::PreferencesStore;
use statlet::preferences_view::{
    IdentifierDetailPresentation, IndicatorControlsLayout, IndicatorControlsVisibility,
    MetricIdentifierControlPresentation,
};
use tempfile::tempdir;

#[test]
fn identifier_defaults_preserve_c_and_r_text_with_metric_specific_symbols_ready() {
    let preferences = IndicatorPreferences::default();

    assert_eq!(preferences.identifiers.cpu.mode, MetricIdentifierMode::Text);
    assert_eq!(preferences.identifiers.ram.mode, MetricIdentifierMode::Text);
    assert_eq!(preferences.identifiers.cpu.system_symbol.as_str(), "cpu");
    assert_eq!(
        preferences.identifiers.ram.system_symbol.as_str(),
        "memorychip"
    );
    assert!(preferences.identifiers.cpu.png.is_none());
    assert!(preferences.identifiers.ram.png.is_none());
    assert_eq!(preferences.labels.cpu.as_str(), "C");
    assert_eq!(preferences.labels.ram.as_str(), "R");
}

#[test]
fn system_symbols_are_limited_to_the_macos_14_curated_catalog() {
    assert_eq!(SystemSymbolName::new("cpu").unwrap().as_str(), "cpu");
    assert_eq!(
        SystemSymbolName::new("  memorychip  ").unwrap().as_str(),
        "memorychip"
    );
    assert!(SystemSymbolName::new("arbitrary.unverified.symbol").is_err());
    assert!(SystemSymbolName::new("").is_err());
    assert!(SystemSymbolName::curated_names().contains(&"gauge.with.dots.needle.33percent"));
}

#[test]
fn identifier_changes_are_metric_scoped_and_redraw_then_save_once() {
    let mut app = StatletCore::new();
    let symbol = SystemSymbolName::new("waveform.path.ecg").unwrap();

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricSystemSymbol {
            metric: MetricKind::Cpu,
            symbol: symbol.clone(),
        },
    ));
    assert_eq!(
        app.state()
            .preferences
            .indicator
            .identifiers
            .cpu
            .system_symbol,
        symbol
    );
    assert_eq!(
        app.state()
            .preferences
            .indicator
            .identifiers
            .ram
            .system_symbol
            .as_str(),
        "memorychip"
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::QueuePreferencesSave(app.state().preferences.clone()),
        ]
    );

    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Cpu,
            mode: MetricIdentifierMode::SystemSymbol,
        },
    ));
    assert_eq!(
        app.state().preferences.indicator.identifiers.cpu.mode,
        MetricIdentifierMode::SystemSymbol
    );
    assert_eq!(effects[0], AppEffect::RequestIndicatorRedraw);
    assert_eq!(
        effects[1],
        AppEffect::QueuePreferencesSave(app.state().preferences.clone())
    );

    assert!(app
        .handle(AppEvent::UpdateIndicator(
            IndicatorPreferenceChange::SetMetricIdentifierMode {
                metric: MetricKind::Cpu,
                mode: MetricIdentifierMode::SystemSymbol,
            },
        ))
        .is_empty());
}

#[test]
fn version_two_without_identifiers_migrates_to_text_defaults() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let mut legacy = serde_json::to_value(stored_defaults(&directory)).unwrap();
    legacy["indicator"]
        .as_object_mut()
        .unwrap()
        .remove("identifiers");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

    let loaded = store.load();

    assert_eq!(
        loaded.indicator.identifiers,
        IndicatorPreferences::default().identifiers
    );
    assert_eq!(loaded.indicator.labels.cpu.as_str(), "C");
    assert_eq!(loaded.indicator.labels.ram.as_str(), "R");
}

#[test]
fn identifier_round_trip_preserves_symbol_and_png_metadata() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let mut expected = Preferences::default();
    expected.indicator.identifiers.cpu.mode = MetricIdentifierMode::SystemSymbol;
    expected.indicator.identifiers.cpu.system_symbol =
        SystemSymbolName::new("gauge.with.dots.needle.33percent").unwrap();
    expected.indicator.identifiers.ram.mode = MetricIdentifierMode::Png;
    expected.indicator.identifiers.ram.png =
        Some(PngIconMetadata::new("ram-custom.png", 24, 16, 812).unwrap());

    store.save(expected.clone()).unwrap();

    assert_eq!(store.load(), expected);
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert_eq!(
        saved["indicator"]["identifiers"]["cpu"]["mode"],
        "systemSymbol"
    );
    assert_eq!(
        saved["indicator"]["identifiers"]["ram"]["png"]["sourceName"],
        "ram-custom.png"
    );
    assert_eq!(saved["indicator"]["identifiers"]["ram"]["png"]["width"], 24);
    assert_eq!(
        saved["indicator"]["identifiers"]["ram"]["png"]["height"],
        16
    );
    assert_eq!(
        saved["indicator"]["identifiers"]["ram"]["png"]["byteLength"],
        812
    );
}

#[test]
fn system_symbol_replaces_only_the_cpu_text_identifier_in_the_surface_spec() {
    let mut preferences = IndicatorPreferences::default();
    preferences.identifiers.cpu.mode = MetricIdentifierMode::SystemSymbol;
    preferences.identifiers.cpu.system_symbol = SystemSymbolName::new("cpu").unwrap();

    let scene = compose_indicator(
        &status(),
        &preferences,
        statlet::indicator_preferences::IndicatorAppearance::Light,
    );
    let surface = scene.surface_spec();

    assert_eq!(
        surface
            .top
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "42%"
    );
    assert_eq!(surface.bottom.runs[0].text, "R ");
    assert_eq!(
        surface.top.identifier,
        Some(MetricIdentifierVisual::SystemSymbol {
            name: SystemSymbolName::new("cpu").unwrap(),
            color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".into(),
        })
    );
    assert_eq!(surface.bottom.identifier, None);
}

#[test]
fn graphical_identifier_keeps_a_compact_stable_prefix_when_the_text_label_is_long() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.cpu = IndicatorLabel::new("Processor").unwrap();
    preferences.labels.spacing = statlet::indicator_preferences::LabelSpacing::try_from(3).unwrap();
    preferences.identifiers.cpu.mode = MetricIdentifierMode::SystemSymbol;

    let scene = compose_indicator(
        &status(),
        &preferences,
        statlet::indicator_preferences::IndicatorAppearance::Light,
    );

    assert_eq!(
        scene.surface_spec().top.identifier,
        Some(MetricIdentifierVisual::SystemSymbol {
            name: SystemSymbolName::new("cpu").unwrap(),
            color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".into(),
        })
    );
    assert_eq!(scene.top[0].text, "42%");
}

#[test]
fn png_surface_keeps_metadata_and_missing_png_metadata_falls_back_to_text() {
    let mut preferences = IndicatorPreferences::default();
    preferences.identifiers.ram.mode = MetricIdentifierMode::Png;
    preferences.identifiers.ram.png =
        Some(PngIconMetadata::new("ram-art.png", 24, 18, 900).unwrap());

    let scene = compose_indicator(
        &status(),
        &preferences,
        statlet::indicator_preferences::IndicatorAppearance::Dark,
    );
    assert_eq!(
        scene.surface_spec().bottom.identifier,
        Some(MetricIdentifierVisual::Png {
            metric: MetricKind::Ram,
            metadata: PngIconMetadata::new("ram-art.png", 24, 18, 900).unwrap(),
            fallback_color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "R ".into(),
        })
    );
    assert_eq!(
        scene
            .bottom
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "68%"
    );

    preferences.identifiers.ram.png = None;
    let fallback = compose_indicator(
        &status(),
        &preferences,
        statlet::indicator_preferences::IndicatorAppearance::Dark,
    );
    assert_eq!(fallback.bottom[0].text, "R ");
    assert_eq!(fallback.surface_spec().bottom.identifier, None);
}

#[test]
fn identifier_controls_get_distinct_rows_before_the_existing_label_editor() {
    let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

    assert!(layout.identifiers_heading().top() < layout.cpu_identifier_row().top());
    assert!(layout.cpu_identifier_row().bottom() < layout.cpu_identifier_detail().top());
    assert!(layout.cpu_identifier_detail().bottom() < layout.ram_identifier_row().top());
    assert!(layout.ram_identifier_row().bottom() < layout.ram_identifier_detail().top());
    assert!(layout.ram_identifier_detail().bottom() < layout.labels_heading().top());
}

#[test]
fn identifier_control_presentation_exposes_only_the_selected_mode_details() {
    let defaults = IndicatorPreferences::default();
    assert_eq!(
        MetricIdentifierControlPresentation::new(&defaults.identifiers.cpu, None).detail,
        IdentifierDetailPresentation::Hidden
    );

    let mut symbol = defaults.identifiers.cpu.clone();
    symbol.mode = MetricIdentifierMode::SystemSymbol;
    assert_eq!(
        MetricIdentifierControlPresentation::new(&symbol, None).detail,
        IdentifierDetailPresentation::SystemSymbol {
            selected_name: "cpu".into()
        }
    );

    let mut png = defaults.identifiers.ram.clone();
    png.mode = MetricIdentifierMode::Png;
    png.png = Some(PngIconMetadata::new("memória.png", 18, 18, 700).unwrap());
    let presentation = MetricIdentifierControlPresentation::new(
        &png,
        Some("Não foi possível abrir este arquivo como PNG."),
    );
    assert_eq!(
        presentation.detail,
        IdentifierDetailPresentation::Png {
            source_name: Some("memória.png".into()),
            can_remove: true,
        }
    );
    assert_eq!(
        presentation.error.as_deref(),
        Some("Não foi possível abrir este arquivo como PNG.")
    );
}

#[test]
fn preview_accessibility_names_the_system_symbol_and_keeps_value_color_order() {
    let mut preferences = IndicatorPreferences::default();
    preferences.identifiers.cpu.mode = MetricIdentifierMode::SystemSymbol;
    let scene = compose_indicator(
        &status(),
        &preferences,
        statlet::indicator_preferences::IndicatorAppearance::Light,
    );
    let colors = [
        [0x11 as f64 / 255.0; 3],
        [0x22 as f64 / 255.0; 3],
        [0x33 as f64 / 255.0; 3],
        [0x44 as f64 / 255.0; 3],
    ];

    assert_eq!(
        preview_accessibility_summary(
            &scene,
            &colors,
            statlet::indicator_preferences::IndicatorAppearance::Light,
        ),
        "Prévia clara: CPU 42%, ícone do macOS cpu na cor #111111 e valor #222222; RAM 68%, rótulo #333333 e valor #444444; identificadores visíveis; badge ausente."
    );
}

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
        accessibility_label: "CPU 42%, RAM 68%".into(),
    }
}

fn stored_defaults(directory: &tempfile::TempDir) -> serde_json::Value {
    let path = directory.path().join("seed.json");
    PreferencesStore::new(path.clone())
        .save(Preferences::default())
        .unwrap();
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
