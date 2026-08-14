use std::path::PathBuf;

use statlet::core::{
    AppEffect, AppEvent, GlobalIndicatorUndoFailure, GlobalIndicatorUndoFailureStage,
    IndicatorPreferenceChange, MetricPngAssetMutation, MetricPngImportResult,
    MetricPngRemovalResult, PreferencesSaveStatus, StatletCore,
};
use statlet::indicator_preferences::IndicatorPreferenceGroup;
use statlet::indicator_preferences::{
    MetricIdentifierMode, MetricKind, PngIconMetadata, SystemSymbolName,
};
use statlet::preferences_view::{PreferencesArea, PreferencesShellPresentation};

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
    assert!(app.state().indicator_icon_pending(MetricKind::Cpu));
    assert!(!app.state().indicator_icon_pending(MetricKind::Ram));
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
    assert!(!app.state().indicator_icon_pending(MetricKind::Cpu));
    assert_eq!(
        effects,
        vec![
            AppEffect::RequestIndicatorRedraw,
            AppEffect::PersistMetricPngChange {
                metric: MetricKind::Cpu,
                mutation: MetricPngAssetMutation::Replace,
                previous: statlet::indicator_preferences::IndicatorPreferences::default()
                    .identifiers
                    .cpu,
                preferences: app.state().preferences.clone(),
            },
        ]
    );
}

#[test]
fn reimporting_equal_metadata_still_replaces_a_missing_or_corrupt_asset() {
    let mut app = StatletCore::new();
    let metadata = PngIconMetadata::new("custom-cpu.png", 24, 12, 812).unwrap();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(metadata.clone()),
    });

    let effects = app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(metadata),
    });

    assert!(matches!(
        effects.as_slice(),
        [
            AppEffect::RequestIndicatorRedraw,
            AppEffect::PersistMetricPngChange {
                mutation: MetricPngAssetMutation::Replace,
                ..
            }
        ]
    ));
}

#[test]
fn failed_preferences_save_rolls_back_imported_metadata_and_exposes_the_failure() {
    let mut app = StatletCore::new();
    let previous = app.state().preferences.indicator.identifiers.cpu.clone();
    let metadata = PngIconMetadata::new("custom-cpu.png", 24, 12, 812).unwrap();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(metadata),
    });

    let effects = app.handle(AppEvent::MetricPngPersistenceFailed {
        metric: MetricKind::Cpu,
        previous: previous.clone(),
        message: "Não foi possível salvar as preferências; o PNG anterior foi restaurado.".into(),
    });

    assert_eq!(app.state().preferences.indicator.identifiers.cpu, previous);
    assert_eq!(
        app.state().preferences_save_status,
        PreferencesSaveStatus::Failed
    );
    assert_eq!(
        app.state().indicator_icon_error(MetricKind::Cpu),
        Some("Não foi possível salvar as preferências; o PNG anterior foi restaurado.")
    );
    assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
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
    assert!(!app.state().indicator_icon_pending(MetricKind::Ram));

    app.handle(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Ram,
            mode: MetricIdentifierMode::SystemSymbol,
        },
    ));
    assert_eq!(app.state().indicator_icon_error(MetricKind::Ram), None);
}

#[test]
fn explicitly_reselecting_the_active_identifier_mode_clears_a_stale_png_error() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Failed("PNG inválido.".into()),
    });

    let effects = app.handle(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Cpu,
            mode: MetricIdentifierMode::Text,
        },
    ));

    assert!(effects.is_empty());
    assert_eq!(app.state().indicator_icon_error(MetricKind::Cpu), None);
}

#[test]
fn changing_identifier_mode_cancels_an_in_flight_png_import() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Cpu,
        source: PathBuf::from("/tmp/slow.png"),
    });

    let effects = app.handle(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Cpu,
            mode: MetricIdentifierMode::SystemSymbol,
        },
    ));

    assert!(!app.state().indicator_icon_pending(MetricKind::Cpu));
    assert_eq!(
        effects[0],
        AppEffect::CancelMetricPngImport(MetricKind::Cpu)
    );
    assert_eq!(effects[1], AppEffect::RequestIndicatorRedraw);
}

#[test]
fn explicitly_reselecting_the_active_mode_cancels_an_in_flight_png_import() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Cpu,
        source: PathBuf::from("/tmp/slow.png"),
    });

    let effects = app.handle(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Cpu,
            mode: MetricIdentifierMode::Text,
        },
    ));

    assert_eq!(
        effects,
        vec![AppEffect::CancelMetricPngImport(MetricKind::Cpu)]
    );
    assert!(!app.state().indicator_icon_pending(MetricKind::Cpu));
}

#[test]
fn choosing_another_png_invalidates_the_in_flight_import_before_starting_the_reselection() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Cpu,
        source: PathBuf::from("/tmp/slow.png"),
    });

    let effects = app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Cpu,
        source: PathBuf::from("/tmp/reselected.png"),
    });

    assert_eq!(
        effects,
        vec![
            AppEffect::CancelMetricPngImport(MetricKind::Cpu),
            AppEffect::ImportMetricPng {
                metric: MetricKind::Cpu,
                source: PathBuf::from("/tmp/reselected.png"),
            },
        ]
    );
    assert!(app.state().indicator_icon_pending(MetricKind::Cpu));
}

#[test]
fn cancelling_the_picker_invalidates_an_existing_in_flight_import() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Ram,
        source: PathBuf::from("/tmp/slow.png"),
    });

    let effects = app.handle(AppEvent::CancelMetricPngImport(MetricKind::Ram));

    assert_eq!(
        effects,
        vec![AppEffect::CancelMetricPngImport(MetricKind::Ram)]
    );
    assert!(!app.state().indicator_icon_pending(MetricKind::Ram));
}

#[test]
fn identifier_group_reset_invalidates_both_pending_imports_even_when_values_are_default() {
    let mut app = StatletCore::new();
    for metric in [MetricKind::Cpu, MetricKind::Ram] {
        app.handle(AppEvent::ChooseMetricPng {
            metric,
            source: PathBuf::from(format!("/tmp/{metric:?}.png")),
        });
    }

    let effects = app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));

    assert_eq!(
        effects,
        vec![
            AppEffect::CancelMetricPngImport(MetricKind::Cpu),
            AppEffect::CancelMetricPngImport(MetricKind::Ram),
        ]
    );
    assert!(!app.state().indicator_icon_pending(MetricKind::Cpu));
    assert!(!app.state().indicator_icon_pending(MetricKind::Ram));
}

#[test]
fn color_reset_does_not_cancel_or_reset_identifiers() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::ChooseMetricPng {
        metric: MetricKind::Cpu,
        source: PathBuf::from("/tmp/cpu.png"),
    });

    let effects = app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::CpuAndRam,
    ));

    assert!(effects.is_empty());
    assert!(app.state().indicator_icon_pending(MetricKind::Cpu));
}

#[test]
fn identifier_group_reset_clears_stale_png_errors_for_both_metrics() {
    let mut app = StatletCore::new();
    for metric in [MetricKind::Cpu, MetricKind::Ram] {
        app.handle(AppEvent::MetricPngImportFinished {
            metric,
            result: MetricPngImportResult::Failed("PNG inválido.".into()),
        });
    }

    let effects = app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));

    assert!(effects.is_empty());
    assert_eq!(app.state().indicator_icon_error(MetricKind::Cpu), None);
    assert_eq!(app.state().indicator_icon_error(MetricKind::Ram), None);
}

#[test]
fn global_reset_invalidates_both_pending_imports_even_when_values_are_default() {
    let mut app = StatletCore::new();
    for metric in [MetricKind::Cpu, MetricKind::Ram] {
        app.handle(AppEvent::ChooseMetricPng {
            metric,
            source: PathBuf::from(format!("/tmp/{metric:?}.png")),
        });
    }

    let effects = app.handle(AppEvent::ResetIndicatorConfirmed);

    assert_eq!(
        effects,
        vec![
            AppEffect::CancelMetricPngImport(MetricKind::Cpu),
            AppEffect::CancelMetricPngImport(MetricKind::Ram),
        ]
    );
    assert!(!app.state().indicator_icon_pending(MetricKind::Cpu));
    assert!(!app.state().indicator_icon_pending(MetricKind::Ram));
}

#[test]
fn global_reset_clears_stale_png_errors_for_both_metrics() {
    let mut app = StatletCore::new();
    for metric in [MetricKind::Cpu, MetricKind::Ram] {
        app.handle(AppEvent::MetricPngImportFinished {
            metric,
            result: MetricPngImportResult::Failed("PNG inválido.".into()),
        });
    }

    let effects = app.handle(AppEvent::ResetIndicatorConfirmed);

    assert!(effects.is_empty());
    assert_eq!(app.state().indicator_icon_error(MetricKind::Cpu), None);
    assert_eq!(app.state().indicator_icon_error(MetricKind::Ram), None);
}

#[test]
fn stale_error_recovery_preserves_unresolved_png_durability_warnings() {
    fn assert_warning_survives(event: AppEvent) {
        let mut app = StatletCore::new();
        app.handle(AppEvent::MetricPngDurabilityWarning {
            metric: MetricKind::Cpu,
            message: "O PNG foi salvo, mas a durabilidade ainda não foi confirmada.".into(),
        });

        app.handle(event);

        assert_eq!(
            app.state().indicator_icon_error(MetricKind::Cpu),
            Some("O PNG foi salvo, mas a durabilidade ainda não foi confirmada.")
        );
    }

    assert_warning_survives(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetMetricIdentifierMode {
            metric: MetricKind::Cpu,
            mode: MetricIdentifierMode::Text,
        },
    ));
    assert_warning_survives(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));
    assert_warning_survives(AppEvent::ResetIndicatorConfirmed);
}

#[test]
fn identifier_reset_restores_both_defaults_and_persists_png_removal_as_one_change() {
    let mut app = StatletCore::new();
    for (metric, name) in [(MetricKind::Cpu, "cpu.png"), (MetricKind::Ram, "ram.png")] {
        app.handle(AppEvent::MetricPngImportFinished {
            metric,
            result: MetricPngImportResult::Imported(
                PngIconMetadata::new(name, 12, 12, 400).unwrap(),
            ),
        });
    }
    let previous = app.state().preferences.indicator.identifiers.clone();

    let effects = app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));

    assert_eq!(
        app.state().preferences.indicator.identifiers,
        statlet::indicator_preferences::IdentifierPreferences::default()
    );
    assert!(matches!(
        effects.as_slice(),
        [
            AppEffect::RequestIndicatorRedraw,
            AppEffect::PersistIdentifierReset {
                previous: actual_previous,
                preferences,
            },
        ] if actual_previous == &previous && preferences == &app.state().preferences
    ));
}

#[test]
fn failed_identifier_reset_persistence_restores_both_identifiers_without_touching_colors() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(
            PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap(),
        ),
    });
    let previous = app.state().preferences.indicator.identifiers.clone();
    let colors = (
        app.state().preferences.indicator.cpu_color,
        app.state().preferences.indicator.ram_color,
    );
    app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));

    let effects = app.handle(AppEvent::IdentifierResetPersistenceFailed {
        previous: previous.clone(),
        message: "Não foi possível salvar as preferências; os identificadores anteriores foram restaurados.".into(),
    });

    assert_eq!(app.state().preferences.indicator.identifiers, previous);
    assert_eq!(
        (
            app.state().preferences.indicator.cpu_color,
            app.state().preferences.indicator.ram_color,
        ),
        colors
    );
    assert_eq!(
        app.state().preferences_save_status,
        PreferencesSaveStatus::Failed
    );
    assert_eq!(effects, vec![AppEffect::RequestIndicatorRedraw]);
}

#[test]
fn identifier_group_reset_does_not_create_a_public_undo() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(
            PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap(),
        ),
    });

    app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));

    assert!(!app.state().can_undo_indicator_reset);
    assert!(app.handle(AppEvent::UndoIndicatorReset).is_empty());
}

#[test]
fn global_reset_and_undo_preserve_png_assets_and_the_existing_snapshot() {
    let mut app = StatletCore::new();
    let metadata = PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(metadata.clone()),
    });
    let before = app.state().preferences.indicator.identifiers.clone();

    let reset_effects = app.handle(AppEvent::ResetIndicatorConfirmed);
    assert!(reset_effects.iter().all(|effect| !matches!(
        effect,
        AppEffect::RemoveMetricPngAsset(_)
            | AppEffect::PersistMetricPngChange { .. }
            | AppEffect::PersistIdentifierReset { .. }
    )));

    app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricSystemSymbol {
            metric: MetricKind::Cpu,
            symbol: SystemSymbolName::new("waveform.path.ecg").unwrap(),
        },
    ));
    app.handle(AppEvent::ResetIndicatorGroup(
        IndicatorPreferenceGroup::Identifiers,
    ));
    assert!(app.state().can_undo_indicator_reset);

    let undo_effects = app.handle(AppEvent::UndoIndicatorReset);

    assert_eq!(app.state().preferences.indicator.identifiers, before);
    assert_eq!(
        app.state().preferences.indicator.identifiers.cpu.png,
        Some(metadata)
    );
    assert!(undo_effects.iter().all(|effect| !matches!(
        effect,
        AppEffect::RemoveMetricPngAsset(_)
            | AppEffect::PersistMetricPngChange { .. }
            | AppEffect::PersistIdentifierReset { .. }
    )));
    assert!(!app.state().can_undo_indicator_reset);
}

#[test]
fn undo_prepare_failure_uses_current_and_undo_assets_and_preserves_undo_retry() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::MetricPngImportFinished {
        metric: MetricKind::Cpu,
        result: MetricPngImportResult::Imported(
            PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap(),
        ),
    });
    app.handle(AppEvent::ResetIndicatorConfirmed);
    let (current, undo) = app
        .handle(AppEvent::UndoIndicatorReset)
        .into_iter()
        .find_map(|effect| match effect {
            AppEffect::PersistGlobalIndicatorUndo { current, undo, .. } => Some((current, undo)),
            _ => None,
        })
        .unwrap();
    let message = "Não foi possível preparar o PNG original para o Undo.".to_owned();
    let save_status = app.state().preferences_save_status;

    app.handle(AppEvent::GlobalIndicatorUndoPersistenceFailed(Box::new(
        GlobalIndicatorUndoFailure {
            current: current.clone(),
            undo,
            message: message.clone(),
            stage: GlobalIndicatorUndoFailureStage::AssetPreparation,
        },
    )));

    assert_eq!(app.state().preferences.indicator, current);
    assert_eq!(
        app.state().indicator_icon_error(MetricKind::Cpu),
        Some(message.as_str())
    );
    assert!(app.state().can_undo_indicator_reset);
    assert_eq!(app.state().preferences_save_status, save_status);
    let shell = PreferencesShellPresentation::new(
        PreferencesArea::Labels,
        app.state().can_undo_indicator_reset,
        app.state().preferences_save_status,
    );
    assert!(shell.undo_visible());
    assert!(!shell.retry_visible());
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
    assert!(matches!(
        &effects[1],
        AppEffect::PersistMetricPngChange {
            metric: MetricKind::Ram,
            mutation: MetricPngAssetMutation::Remove,
            preferences,
            ..
        } if preferences == &app.state().preferences
    ));
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
