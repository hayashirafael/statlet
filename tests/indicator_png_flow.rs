use std::path::PathBuf;

use statlet::core::{
    AppEffect, AppEvent, MetricPngImportResult, MetricPngRemovalResult, StatletCore,
};
use statlet::indicator_preferences::{MetricIdentifierMode, MetricKind, PngIconMetadata};

#[test]
fn choosing_a_png_requests_import_without_committing_preferences_early() {
    let mut app = StatletCore::new();
    let before = app.state().preferences.clone();
    let source = PathBuf::from("/tmp/custom-cpu.png");

    let effects = app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Cpu,
        source: source.clone(),
    });

    assert_eq!(
        effects,
        vec![AppEffect::ImportMetricPng {
            metric: MetricKind::Cpu,
            source
        }]
    );
    assert_eq!(app.state().preferences, before);
}

#[test]
fn successful_import_commits_png_mode_redraws_and_saves_the_document() {
    let mut app = StatletCore::new();
    let metadata = PngIconMetadata::new("custom-cpu.png", 24, 12, 812).unwrap();

    let effects = app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(metadata.clone()),
    });

    assert_eq!(
        app.state().preferences.indicator.identifiers.cpu.mode,
        MetricIdentifierMode::Png
    );
    assert_eq!(
        app.state().preferences.indicator.identifiers.cpu.png,
        Some(metadata)
    );
    assert_eq!(app.state().indicator_icon_error(MetricKind::Cpu), None);
    assert_eq!(
        effects,
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::QueuePreferencesSave(app.state().preferences.clone()),
        ]
    );
}

#[test]
fn failed_import_keeps_the_previous_mode_and_exposes_a_pt_br_error() {
    let mut app = StatletCore::new();
    let before = app.state().preferences.clone();

    let effects = app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Ram,
        result: MetricPngImportResult::Failed(
            "Não foi possível abrir este arquivo como PNG.".into(),
        ),
    });

    assert!(effects.is_empty());
    assert_eq!(app.state().preferences, before);
    assert_eq!(
        app.state().indicator_icon_error(MetricKind::Ram),
        Some("Não foi possível abrir este arquivo como PNG.")
    );
    assert_eq!(app.state().indicator_icon_error(MetricKind::Cpu), None);

    app.handle(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Ram,
            mode: MetricIdentifierMode::SystemSymbol,
        },
    ));
    assert_eq!(app.state().indicator_icon_error(MetricKind::Ram), None);
}

#[test]
fn removal_is_committed_only_after_the_asset_store_succeeds() {
    let mut app = StatletCore::new();
    let metadata = PngIconMetadata::new("ram.png", 12, 12, 400).unwrap();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Ram,
        result: MetricPngImportResult::Imported(metadata),
    });

    assert_eq!(
        app.handle(AppEvent::RemoveMetricPng(MetricKind::Ram)),
        vec![AppEffect::RemoveMetricPngAsset(MetricKind::Ram)]
    );
    assert_eq!(
        app.state().preferences.indicator.identifiers.ram.mode,
        MetricIdentifierMode::Png
    );

    let effects = app.handle(AppEvent::MetricPngRemovalFinished {
        metric: MetricKind::Ram,
        result: MetricPngRemovalResult::Removed,
    });
    assert_eq!(
        app.state().preferences.indicator.identifiers.ram.mode,
        MetricIdentifierMode::Text
    );
    assert!(app
        .state()
        .preferences
        .indicator
        .identifiers
        .ram
        .png
        .is_none());
    assert_eq!(effects[0], AppEffect::RequestIndicatorRedraw);
    assert_eq!(
        effects[1],
        AppEffect::QueuePreferencesSave(app.state().preferences.clone())
    );
}

#[test]
fn failed_removal_preserves_png_and_reports_the_failure() {
    let mut app = StatletCore::new();
    let metadata = PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(metadata.clone()),
    });

    let effects = app.handle(AppEvent::MetricPngRemovalFinished {
        metric: MetricKind::Cpu,
        result: MetricPngRemovalResult::Failed("Não foi possível remover o PNG.".into()),
    });

    assert!(effects.is_empty());
    assert_eq!(
        app.state().preferences.indicator.identifiers.cpu.png,
        Some(metadata)
    );
    assert_eq!(
        app.state().indicator_icon_error(MetricKind::Cpu),
        Some("Não foi possível remover o PNG.")
    );
}
