use statlet::indicator_preferences::{
    AppearanceColors, FixedColorPreferences, MetricsRefreshInterval, SrgbColor,
};
use statlet::preferences_view::{
    color_well_configuration, filter_font_families, ColorEditorFocusTarget, ColorEditorRows,
    ColorEditorState, ColorWellPresentation, FontPickerInteraction, FontRow, HexDraft,
    HexDraftError, HexEdit, IntervalDraft,
};

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

    app.handle(AppEvent::ResetIndicatorConfirmed);
    assert!(cache.should_apply(app.state()));

    app.handle(AppEvent::PreferencesSaveFinished(
        PreferencesSaveResult::Failed,
    ));
    assert!(cache.should_apply(app.state()));
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
