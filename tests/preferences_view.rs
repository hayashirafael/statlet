use statlet::indicator_preferences::{AppearanceColors, FixedColorPreferences, SrgbColor};
use statlet::preferences_view::{
    ColorEditorRows, ColorEditorState, HexDraft, HexDraftError, HexEdit,
};

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
fn six_valid_digits_apply_and_normalize_immediately() {
    let mut draft = HexDraft::new(SrgbColor::parse_hex("#34C759").unwrap());
    assert_eq!(
        draft.edit("0a84ff"),
        HexEdit::Applied(SrgbColor::parse_hex("#0A84FF").unwrap())
    );
    assert_eq!(draft.text(), "#0A84FF");
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
