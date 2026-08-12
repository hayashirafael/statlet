use crate::indicator_preferences::{AppearanceColors, FixedColorPreferences, SrgbColor};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HexEdit {
    Incomplete,
    Invalid,
    Applied(SrgbColor),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HexDraftError {
    ExpectedSixDigits,
    InvalidDigit,
}

impl HexDraftError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::ExpectedSixDigits => "Use exatamente 6 dígitos hexadecimais.",
            Self::InvalidDigit => "Use somente dígitos de 0–9 e letras de A–F.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HexDraft {
    text: String,
    valid_color: SrgbColor,
    error: Option<HexDraftError>,
}

impl HexDraft {
    pub fn new(color: SrgbColor) -> Self {
        Self {
            text: color.to_hex(),
            valid_color: color,
            error: None,
        }
    }

    pub fn edit(&mut self, text: &str) -> HexEdit {
        self.text = text.to_owned();
        self.error = None;

        let digits = text.strip_prefix('#').unwrap_or(text);
        if digits.len() != 6 {
            return HexEdit::Incomplete;
        }

        match SrgbColor::parse_hex(digits) {
            Ok(color) => {
                self.valid_color = color;
                self.text = color.to_hex();
                HexEdit::Applied(color)
            }
            Err(_) => HexEdit::Invalid,
        }
    }

    pub fn commit(&mut self) -> Result<SrgbColor, HexDraftError> {
        let digits = self.text.strip_prefix('#').unwrap_or(&self.text);
        let result = if digits.len() != 6 {
            Err(HexDraftError::ExpectedSixDigits)
        } else {
            SrgbColor::parse_hex(digits).map_err(|_| HexDraftError::InvalidDigit)
        };

        match result {
            Ok(color) => {
                self.valid_color = color;
                self.text = color.to_hex();
                self.error = None;
                Ok(color)
            }
            Err(error) => {
                self.error = Some(error);
                Err(error)
            }
        }
    }

    pub fn set_color(&mut self, color: SrgbColor) {
        self.valid_color = color;
        self.text = color.to_hex();
        self.error = None;
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn valid_color(&self) -> SrgbColor {
        self.valid_color
    }

    pub const fn error(&self) -> Option<HexDraftError> {
        self.error
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorEditorState {
    variants_enabled: bool,
    shared: HexDraft,
    light: HexDraft,
    dark: HexDraft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorEditorRows {
    Shared,
    Appearances,
}

impl ColorEditorState {
    pub fn from_preferences(preferences: FixedColorPreferences) -> Self {
        let variants = preferences.variants.unwrap_or(AppearanceColors {
            light: preferences.shared,
            dark: preferences.shared,
        });
        Self {
            variants_enabled: preferences.use_appearance_variants,
            shared: HexDraft::new(preferences.shared),
            light: HexDraft::new(variants.light),
            dark: HexDraft::new(variants.dark),
        }
    }

    pub const fn variants_enabled(&self) -> bool {
        self.variants_enabled
    }

    pub fn set_variants_enabled(&mut self, enabled: bool) {
        self.variants_enabled = enabled;
    }

    pub fn sync_from_preferences(&mut self, preferences: FixedColorPreferences) {
        self.sync_from(&Self::from_preferences(preferences));
    }

    pub fn sync_from(&mut self, state: &Self) {
        self.variants_enabled = state.variants_enabled;
        sync_draft(&mut self.shared, state.shared.valid_color());
        sync_draft(&mut self.light, state.light.valid_color());
        sync_draft(&mut self.dark, state.dark.valid_color());
    }

    pub const fn shared_row_visible(&self) -> bool {
        !self.variants_enabled
    }

    pub const fn appearance_rows_visible(&self) -> bool {
        self.variants_enabled
    }

    pub const fn visible_rows(&self) -> ColorEditorRows {
        if self.variants_enabled {
            ColorEditorRows::Appearances
        } else {
            ColorEditorRows::Shared
        }
    }

    pub const fn shared(&self) -> &HexDraft {
        &self.shared
    }

    pub const fn light(&self) -> &HexDraft {
        &self.light
    }

    pub const fn dark(&self) -> &HexDraft {
        &self.dark
    }

    pub fn shared_mut(&mut self) -> &mut HexDraft {
        &mut self.shared
    }

    pub fn light_mut(&mut self) -> &mut HexDraft {
        &mut self.light
    }

    pub fn dark_mut(&mut self) -> &mut HexDraft {
        &mut self.dark
    }
}

fn sync_draft(draft: &mut HexDraft, persisted: SrgbColor) {
    if draft.valid_color() != persisted {
        draft.set_color(persisted);
    }
}
