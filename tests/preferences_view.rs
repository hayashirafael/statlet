use statlet::indicator_preferences::{
    AppearanceColors, FixedColorPreferences, MetricIdentifierMode, MetricsRefreshInterval,
    SrgbColor,
};
use statlet::preferences_view::{
    color_well_configuration, filter_font_families, ColorEditorFocusTarget, ColorEditorRows,
    ColorEditorState, ColorWellPresentation, FontPickerInteraction, FontRow,
    GeneralPreferencesPresentation, HexDraft, HexDraftError, HexEdit, IdentifierEditingFocusTarget,
    IdentifierEditingPresentation, IndicatorControlsLayout, IndicatorControlsVisibility,
    IntervalDraft, IntervalFieldFormat, LabelEditingFocusTarget, LabelEditingPresentation,
    MessageLayout, PreferencesArea, PreferencesNavigationArea, PreferencesNavigationPolicy,
    PreferencesShellFocusTarget, PreferencesShellPresentation, TypographyWarningKind,
};

#[test]
fn general_recovery_layout_wraps_complete_help_in_two_lines() {
    let (width, height, max_lines) = statlet::preferences_view::general_recovery_layout();
    assert_eq!((width, height, max_lines), (400.0, 44.0, 2));
    let help =
        GeneralPreferencesPresentation::from_preferences(&statlet::core::Preferences::default())
            .recovery_help();
    assert!(help.contains("voltar às Preferências."));
    assert!(help.len() > 24);
}

#[test]
fn general_area_leads_the_navigation_with_recoverable_menu_bar_copy() {
    use statlet::core::Preferences;

    let areas = (0..=5)
        .map(|row| PreferencesNavigationArea::from_sidebar_row(row).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        areas,
        vec![
            PreferencesNavigationArea::General,
            PreferencesNavigationArea::Colors,
            PreferencesNavigationArea::Labels,
            PreferencesNavigationArea::Typography,
            PreferencesNavigationArea::Refresh,
            PreferencesNavigationArea::DiskAndMole,
        ]
    );
    assert_eq!(
        PreferencesNavigationArea::default(),
        PreferencesNavigationArea::General
    );
    assert_eq!(PreferencesNavigationArea::General.sidebar_label(), "Geral");
    let switched = PreferencesNavigationPolicy::between(
        PreferencesNavigationArea::General,
        PreferencesNavigationArea::Colors,
    );
    assert_eq!(switched.scroll_origin_y(180.0, 344.0, 648.0, 820.0), 476.0);

    let presentation = GeneralPreferencesPresentation::from_preferences(&Preferences::default());
    assert_eq!(presentation.title(), "Geral");
    assert_eq!(presentation.toggle_identifier(), "general.show-in-menu-bar");
    assert!(presentation.show_in_menu_bar());
    assert_eq!(
        presentation.toggle_label(),
        "Mostrar o Statlet na barra de menus"
    );
    assert_eq!(
        presentation.recovery_help(),
        "Se ocultar o Statlet, abra-o pelo Finder ou Spotlight para voltar às Preferências."
    );

    let hidden = GeneralPreferencesPresentation::from_preferences(&Preferences {
        show_in_menu_bar: false,
        ..Preferences::default()
    });
    assert!(!hidden.show_in_menu_bar());
}

#[test]
fn font_filter_is_case_insensitive_sorted_and_keeps_missing_selection_visible() {
    let result = filter_font_families(
        &["Menlo".into(), "Avenir Next".into()],
        "ave",
        Some("Missing Family"),
    );

    assert_eq!(
        result,
        vec![
            FontRow::Missing("Missing Family".into()),
            FontRow::Available("Avenir Next".into()),
        ]
    );
}

#[test]
fn font_picker_only_confirms_explicit_activation_of_the_selected_row() {
    let preselected_row = 4;

    assert_eq!(
        FontPickerInteraction::NavigateTo(preselected_row + 1).confirmed_row(),
        None
    );
    assert_eq!(
        FontPickerInteraction::Activate(preselected_row).confirmed_row(),
        Some(preselected_row)
    );
}

#[test]
fn interval_draft_applies_only_whole_values_from_one_through_sixty() {
    let mut draft = IntervalDraft::new(MetricsRefreshInterval::default());

    assert!(draft.commit("0").is_err());
    assert!(draft.commit("1.5").is_err());
    assert_eq!(draft.commit("60").unwrap().seconds(), 60);
    assert!(draft.commit("61").is_err());
}

#[test]
fn label_editing_tracks_each_metric_mode_and_skips_disabled_controls() {
    use LabelEditingFocusTarget::{CpuLabel, LabelColorMode, RamLabel, Spacing};

    let graphical = [
        MetricIdentifierMode::SystemSymbol,
        MetricIdentifierMode::Png,
    ];
    let text_text =
        LabelEditingPresentation::new(MetricIdentifierMode::Text, MetricIdentifierMode::Text);
    assert_eq!(
        (text_text.cpu_enabled(), text_text.ram_enabled()),
        (true, true)
    );
    assert!(text_text.spacing_enabled());
    assert_eq!(
        text_text.focus_order(),
        vec![CpuLabel, RamLabel, Spacing, LabelColorMode]
    );

    for non_text in graphical {
        let cpu_only = LabelEditingPresentation::new(MetricIdentifierMode::Text, non_text);
        assert_eq!(
            (cpu_only.cpu_enabled(), cpu_only.ram_enabled()),
            (true, false)
        );
        assert!(cpu_only.spacing_enabled());
        assert_eq!(
            cpu_only.focus_order(),
            vec![CpuLabel, Spacing, LabelColorMode]
        );

        let ram_only = LabelEditingPresentation::new(non_text, MetricIdentifierMode::Text);
        assert_eq!(
            (ram_only.cpu_enabled(), ram_only.ram_enabled()),
            (false, true)
        );
        assert!(ram_only.spacing_enabled());
        assert_eq!(
            ram_only.focus_order(),
            vec![RamLabel, Spacing, LabelColorMode]
        );
    }

    for cpu_mode in graphical {
        for ram_mode in graphical {
            let neither = LabelEditingPresentation::new(cpu_mode, ram_mode);
            assert_eq!(
                (neither.cpu_enabled(), neither.ram_enabled()),
                (false, false)
            );
            assert!(!neither.spacing_enabled());
            assert_eq!(neither.focus_order(), vec![LabelColorMode]);
            assert_eq!(
                (
                    neither.cpu_help(),
                    neither.ram_help(),
                    neither.spacing_help()
                ),
                (
                    Some("Preservado para o modo Texto."),
                    Some("Preservado para o modo Texto."),
                    Some("Preservado para o modo Texto.")
                )
            );
        }
    }
}

#[test]
fn shared_symbol_size_stays_visible_but_is_enabled_only_for_a_system_symbol() {
    let neither =
        IdentifierEditingPresentation::new(MetricIdentifierMode::Text, MetricIdentifierMode::Png);
    assert!(!neither.system_symbol_size_enabled());
    assert_eq!(
        neither.system_symbol_size_help(),
        "Ajusta o tamanho compartilhado dos ícones do macOS de CPU e RAM."
    );

    for modes in [
        (
            MetricIdentifierMode::SystemSymbol,
            MetricIdentifierMode::Text,
        ),
        (
            MetricIdentifierMode::Png,
            MetricIdentifierMode::SystemSymbol,
        ),
    ] {
        assert!(IdentifierEditingPresentation::new(modes.0, modes.1).system_symbol_size_enabled());
    }
}

#[test]
fn identifier_focus_order_skips_hidden_details_and_disabled_shared_size() {
    use IdentifierEditingFocusTarget::{
        CpuMode, CpuSymbol, RamChoosePng, RamMode, RamRemovePng, ResetIdentifiers, SystemSymbolSize,
    };

    let presentation = IdentifierEditingPresentation::new(
        MetricIdentifierMode::SystemSymbol,
        MetricIdentifierMode::Png,
    );
    assert_eq!(
        presentation.focus_order(false, false),
        vec![
            CpuMode,
            CpuSymbol,
            RamMode,
            RamChoosePng,
            SystemSymbolSize,
            ResetIdentifiers,
        ]
    );
    assert_eq!(
        presentation.focus_order(false, true),
        vec![
            CpuMode,
            CpuSymbol,
            RamMode,
            RamChoosePng,
            RamRemovePng,
            SystemSymbolSize,
            ResetIdentifiers,
        ]
    );

    let disabled =
        IdentifierEditingPresentation::new(MetricIdentifierMode::Text, MetricIdentifierMode::Png);
    assert!(!disabled
        .focus_order(false, false)
        .contains(&SystemSymbolSize));
}

#[test]
fn typography_warnings_expose_stable_ax_identity_only_while_visible() {
    let fallback = TypographyWarningKind::FontFallback;
    let layout = TypographyWarningKind::Layout;

    assert_eq!(
        fallback.accessibility_identifier(),
        "indicator.font.fallback-warning"
    );
    assert_eq!(
        layout.accessibility_identifier(),
        "indicator.font.layout-warning"
    );
    assert_eq!(
        fallback.accessibility_label(Some("Fonte substituída")),
        Some("Fonte substituída")
    );
    assert_eq!(layout.accessibility_label(None), None);
}

#[test]
fn refresh_interval_field_uses_an_integer_native_format_from_one_through_sixty() {
    let format = IntervalFieldFormat::seconds();

    assert_eq!((format.minimum(), format.maximum()), (1, 60));
    assert!(!format.allows_floats());
    assert!(!format.uses_grouping_separator());
    assert!(format.validates_partial_input());
    assert!(format.accepts_invalid_commit_for_domain_validation());
}

#[test]
fn interval_commit_matrix_preserves_the_last_valid_value_on_every_invalid_draft() {
    let initial = MetricsRefreshInterval::try_from(2).unwrap();

    for invalid in ["", "0", "61", "abc", "1.5", "-1", "256"] {
        let mut draft = IntervalDraft::new(initial);
        assert!(
            draft.commit(invalid).is_err(),
            "{invalid:?} must be invalid"
        );
        assert_eq!(draft.text(), invalid);
        assert_eq!(draft.valid_interval(), initial);
        assert_eq!(
            draft.error().map(|error| error.message()),
            Some("Digite um número inteiro de 1 a 60.".to_owned())
        );
    }

    for valid in [("1", 1), (" 2 ", 2), ("60", 60)] {
        let mut draft = IntervalDraft::new(initial);
        assert_eq!(draft.commit(valid.0).unwrap().seconds(), valid.1);
        assert_eq!(draft.text(), valid.1.to_string());
        assert_eq!(draft.error(), None);
    }
}

#[test]
fn incomplete_or_invalid_draft_keeps_the_last_valid_color() {
    let valid = SrgbColor::parse_hex("#34C759").unwrap();
    let mut draft = HexDraft::new(valid);
    assert_eq!(draft.edit("#34C7"), HexEdit::Incomplete);
    assert_eq!(draft.valid_color(), valid);
    assert_eq!(draft.commit(), Err(HexDraftError::ExpectedSixDigits));
    assert_eq!(draft.error(), Some(HexDraftError::ExpectedSixDigits));
    assert_eq!(draft.valid_color(), valid);
}

#[test]
fn invalid_characters_are_invalid_even_before_six_digits() {
    let valid = SrgbColor::parse_hex("#34C759").unwrap();
    let mut draft = HexDraft::new(valid);

    assert_eq!(draft.edit("#GG"), HexEdit::Invalid);
    assert_eq!(draft.valid_color(), valid);
    assert_eq!(draft.commit(), Err(HexDraftError::InvalidDigit));
    assert_eq!(draft.error(), Some(HexDraftError::InvalidDigit));
    assert_eq!(draft.valid_color(), valid);

    assert_eq!(draft.edit("#12GG56"), HexEdit::Invalid);
    assert_eq!(draft.valid_color(), valid);
    assert_eq!(draft.commit(), Err(HexDraftError::InvalidDigit));
    assert_eq!(draft.error(), Some(HexDraftError::InvalidDigit));
    assert_eq!(draft.valid_color(), valid);
}

#[test]
fn more_than_six_digits_are_invalid_and_keep_the_last_valid_color() {
    let valid = SrgbColor::parse_hex("#34C759").unwrap();
    let mut draft = HexDraft::new(valid);

    assert_eq!(draft.edit("#1234567"), HexEdit::Invalid);
    assert_eq!(draft.valid_color(), valid);
    assert_eq!(draft.commit(), Err(HexDraftError::ExpectedSixDigits));
    assert_eq!(draft.error(), Some(HexDraftError::ExpectedSixDigits));
    assert_eq!(draft.valid_color(), valid);
}

#[test]
fn six_valid_digits_apply_and_normalize_immediately() {
    let mut draft = HexDraft::new(SrgbColor::parse_hex("#34C759").unwrap());
    assert_eq!(
        draft.edit("0a84ff"),
        HexEdit::Applied(SrgbColor::parse_hex("#0A84FF").unwrap())
    );
    assert_eq!(draft.text(), "#0A84FF");
}

#[test]
fn native_color_well_contract_is_minimal_without_alpha() {
    let configuration = color_well_configuration();

    assert_eq!(configuration.presentation(), ColorWellPresentation::Minimal);
    assert!(!configuration.supports_alpha());
}

#[test]
fn appearance_drafts_survive_collapsing_and_reopening_variants() {
    let shared = SrgbColor::parse_hex("#34C759").unwrap();
    let light = SrgbColor::parse_hex("#0A84FF").unwrap();
    let dark = SrgbColor::parse_hex("#AF52DE").unwrap();
    let mut state = ColorEditorState::from_preferences(FixedColorPreferences {
        shared,
        use_appearance_variants: true,
        variants: Some(AppearanceColors { light, dark }),
    });

    assert_eq!(state.visible_rows(), ColorEditorRows::Appearances);
    state.set_variants_enabled(false);
    assert_eq!(state.visible_rows(), ColorEditorRows::Shared);
    state.set_variants_enabled(true);

    assert_eq!(state.visible_rows(), ColorEditorRows::Appearances);
    assert_eq!(state.light().valid_color(), light);
    assert_eq!(state.dark().valid_color(), dark);
}

#[test]
fn tab_order_visits_each_visible_well_and_hex_then_the_next_group() {
    use ColorEditorFocusTarget::{
        DarkHex, DarkWell, LightHex, LightWell, Mode, NextGroup, SharedHex, SharedWell,
        VariantsToggle,
    };

    let shared = SrgbColor::parse_hex("#34C759").unwrap();
    let mut state = ColorEditorState::from_preferences(FixedColorPreferences {
        shared,
        use_appearance_variants: false,
        variants: Some(AppearanceColors {
            light: SrgbColor::parse_hex("#0A84FF").unwrap(),
            dark: SrgbColor::parse_hex("#AF52DE").unwrap(),
        }),
    });

    assert_eq!(
        state.tab_order(),
        &[Mode, SharedWell, SharedHex, VariantsToggle, NextGroup]
    );

    state.set_variants_enabled(true);
    assert_eq!(
        state.tab_order(),
        &[
            Mode,
            LightWell,
            LightHex,
            DarkWell,
            DarkHex,
            VariantsToggle,
            NextGroup,
        ]
    );
}

#[test]
fn metrics_only_ticks_do_not_change_the_preferences_controls_presentation() {
    use statlet::core::{AppEvent, MemoryPressure, StatletCore, SystemSnapshot};
    use statlet::preferences_view::PreferencesControlsCache;

    let mut app = StatletCore::new();
    let mut cache = PreferencesControlsCache::default();
    assert!(cache.should_apply(app.state()));

    app.handle(AppEvent::MetricsSample(SystemSnapshot {
        cpu_percent: 81.0,
        ram_percent: 64.0,
        memory_pressure: MemoryPressure::Warning,
    }));

    assert!(!cache.should_apply(app.state()));
}

#[test]
fn relevant_preferences_state_changes_the_controls_presentation() {
    use statlet::core::{AppEvent, PreferencesSaveResult, StatletCore};
    use statlet::preferences_view::PreferencesControlsCache;

    let mut app = StatletCore::new();
    let mut cache = PreferencesControlsCache::default();
    assert!(cache.should_apply(app.state()));

    app.handle(AppEvent::SetMoleIntegrationEnabled(true));
    assert!(cache.should_apply(app.state()));

    app.handle(AppEvent::UpdateIndicator(
        statlet::core::IndicatorPreferenceChange::SetFontSize(
            statlet::indicator_preferences::FontSize::try_from(14).unwrap(),
        ),
    ));
    assert!(cache.should_apply(app.state()));

    app.handle(AppEvent::ResetIndicatorConfirmed);
    assert!(cache.should_apply(app.state()));

    app.handle(AppEvent::PreferencesSaveFinished(
        PreferencesSaveResult::Failed,
    ));
    assert!(cache.should_apply(app.state()));
}

#[test]
fn save_recovery_is_visible_and_keyboard_reachable_from_disk_and_mole() {
    use statlet::core::PreferencesSaveStatus;

    let failed = PreferencesShellPresentation::new(
        PreferencesArea::DiskAndMole,
        false,
        PreferencesSaveStatus::Failed,
    );

    assert_eq!(
        failed.save_error(),
        Some("Não foi possível salvar as preferências.")
    );
    assert!(failed.retry_visible());
    assert!(!failed.indicator_reset_visible());
    assert_eq!(
        failed.focus_target_after_area_controls(),
        PreferencesShellFocusTarget::RetrySave
    );

    let saved = PreferencesShellPresentation::new(
        PreferencesArea::DiskAndMole,
        false,
        PreferencesSaveStatus::Saved,
    );
    assert_eq!(saved.save_error(), None);
    assert!(!saved.retry_visible());
    assert_eq!(
        saved.focus_target_after_area_controls(),
        PreferencesShellFocusTarget::Sidebar
    );
}

#[test]
fn area_navigation_reveals_the_new_page_but_same_area_reflow_preserves_scroll() {
    let switched =
        PreferencesNavigationPolicy::between(PreferencesArea::Labels, PreferencesArea::Colors);
    assert_eq!(switched.scroll_origin_y(180.0, 344.0, 648.0, 820.0), 476.0);

    let same_area =
        PreferencesNavigationPolicy::between(PreferencesArea::Labels, PreferencesArea::Labels);
    assert_eq!(same_area.scroll_origin_y(180.0, 344.0, 648.0, 820.0), 352.0);
}

#[test]
fn identifier_reset_has_its_own_row_before_label_controls() {
    let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

    assert!(layout.system_symbol_size_row().top() >= layout.ram_identifier_detail().bottom());
    assert!(layout.identifiers_reset().top() >= layout.system_symbol_size_row().bottom());
    assert!(layout.labels_heading().top() >= layout.identifiers_reset().bottom());
}

#[test]
fn labels_page_translation_keeps_identifier_reset_inside_its_visible_group() {
    let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());
    let reset = layout.identifiers_reset().vertical();
    let reset_y = layout.labels_page_origin_y(reset);
    let heading_y = layout.labels_page_origin_y(layout.identifiers_heading());
    let labels_y = layout.labels_page_origin_y(layout.labels_heading());

    assert!(reset_y >= 0.0);
    assert!(reset_y + reset.height() <= layout.page_height());
    assert!(reset_y + reset.height() <= heading_y);
    assert!(labels_y + layout.labels_heading().height() <= reset_y);
}

#[test]
fn transaction_errors_get_dedicated_wrapped_rows_and_reflow_following_content() {
    let compact = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());
    let failed = IndicatorControlsLayout::new(IndicatorControlsVisibility {
        cpu_identifier_error: true,
        ram_identifier_error: true,
        ..IndicatorControlsVisibility::default()
    });
    let cpu_error = failed.cpu_identifier_error().expect("CPU error row");
    let ram_error = failed.ram_identifier_error().expect("RAM error row");
    let message = MessageLayout::identifier_transaction_error();

    assert!(message.wraps());
    assert!(message.maximum_lines() >= 2);
    assert!(message.width() >= 400.0);
    assert!(cpu_error.height() >= message.height());
    assert!(cpu_error.top() >= failed.cpu_identifier_detail().bottom());
    assert!(failed.ram_identifier_row().top() >= cpu_error.bottom());
    assert!(ram_error.top() >= failed.ram_identifier_detail().bottom());
    assert!(failed.identifiers_reset().top() >= ram_error.bottom());
    assert!(failed.page_height() > compact.page_height());
}

#[test]
fn global_save_error_uses_two_line_wrapping_above_retry() {
    let message = MessageLayout::preferences_save_error();

    assert!(message.wraps());
    assert!(message.maximum_lines() >= 2);
    assert!(message.height() >= 32.0);
    assert!(message.width() >= 200.0);
}

#[test]
fn programmatic_sync_preserves_a_draft_when_the_persisted_color_is_unchanged() {
    let shared = SrgbColor::parse_hex("#34C759").unwrap();
    let preferences = FixedColorPreferences {
        shared,
        use_appearance_variants: false,
        variants: None,
    };
    let mut state = ColorEditorState::from_preferences(preferences);
    assert_eq!(state.shared_mut().edit("#34C7"), HexEdit::Incomplete);

    state.sync_from_preferences(FixedColorPreferences {
        use_appearance_variants: true,
        ..preferences
    });

    assert_eq!(state.shared().text(), "#34C7");
    assert_eq!(state.shared().valid_color(), shared);
    assert_eq!(state.visible_rows(), ColorEditorRows::Appearances);
}
