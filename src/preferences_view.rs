use crate::indicator_preferences::{
    AppearanceColors, FixedColorPreferences, MetricsRefreshInterval, SrgbColor,
};

const SYSTEM_MONOSPACED_LABEL: &str = "System Monospaced";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontRow {
    SystemMonospaced,
    Available(String),
    Missing(String),
}

impl FontRow {
    pub fn family_preference(&self) -> crate::indicator_preferences::FontFamilyPreference {
        match self {
            Self::SystemMonospaced => {
                crate::indicator_preferences::FontFamilyPreference::SystemMonospaced
            }
            Self::Available(family) | Self::Missing(family) => {
                crate::indicator_preferences::FontFamilyPreference::Named(family.clone())
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::SystemMonospaced => SYSTEM_MONOSPACED_LABEL,
            Self::Available(family) | Self::Missing(family) => family,
        }
    }

    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing(_))
    }
}

pub fn filter_font_families(
    families: &[String],
    query: &str,
    missing_selection: Option<&str>,
) -> Vec<FontRow> {
    let query = query.trim().to_lowercase();
    let matches_query = |family: &str| family.to_lowercase().contains(&query);
    let has_selected_family = missing_selection.is_some_and(|selected| {
        let selected = selected.to_lowercase();
        families
            .iter()
            .any(|family| family.to_lowercase() == selected)
    });
    let mut rows = Vec::with_capacity(families.len() + 2);

    if matches_query(SYSTEM_MONOSPACED_LABEL) {
        rows.push(FontRow::SystemMonospaced);
    }
    if !has_selected_family {
        if let Some(selected) = missing_selection {
            rows.push(FontRow::Missing(selected.to_owned()));
        }
    }

    let mut available = families
        .iter()
        .filter(|family| matches_query(family))
        .cloned()
        .collect::<Vec<_>>();
    available.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    rows.extend(available.into_iter().map(FontRow::Available));
    rows
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidIntervalDraft;

impl InvalidIntervalDraft {
    pub const fn message(self) -> &'static str {
        "Digite um número inteiro de 1 a 60."
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntervalDraft {
    text: String,
    valid_interval: MetricsRefreshInterval,
    error: Option<InvalidIntervalDraft>,
}

impl IntervalDraft {
    pub fn new(interval: MetricsRefreshInterval) -> Self {
        Self {
            text: interval.seconds().to_string(),
            valid_interval: interval,
            error: None,
        }
    }

    pub fn commit(&mut self, text: &str) -> Result<MetricsRefreshInterval, InvalidIntervalDraft> {
        self.text = text.to_owned();
        let interval = text
            .trim()
            .parse::<u8>()
            .ok()
            .and_then(|seconds| MetricsRefreshInterval::try_from(seconds).ok())
            .ok_or(InvalidIntervalDraft);

        match interval {
            Ok(interval) => {
                self.valid_interval = interval;
                self.text = interval.seconds().to_string();
                self.error = None;
                Ok(interval)
            }
            Err(error) => {
                self.error = Some(error);
                Err(error)
            }
        }
    }

    pub fn sync(&mut self, interval: MetricsRefreshInterval) {
        if self.valid_interval != interval {
            self.valid_interval = interval;
            self.text = interval.seconds().to_string();
            self.error = None;
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn valid_interval(&self) -> MetricsRefreshInterval {
        self.valid_interval
    }

    pub const fn error(&self) -> Option<InvalidIntervalDraft> {
        self.error
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorWellPresentation {
    Minimal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColorWellConfiguration {
    presentation: ColorWellPresentation,
    supports_alpha: bool,
}

impl ColorWellConfiguration {
    pub const fn presentation(self) -> ColorWellPresentation {
        self.presentation
    }

    pub const fn supports_alpha(self) -> bool {
        self.supports_alpha
    }
}

pub const fn color_well_configuration() -> ColorWellConfiguration {
    ColorWellConfiguration {
        presentation: ColorWellPresentation::Minimal,
        supports_alpha: false,
    }
}

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
        if !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            return HexEdit::Invalid;
        }
        if digits.len() < 6 {
            return HexEdit::Incomplete;
        }
        if digits.len() > 6 {
            return HexEdit::Invalid;
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
        let result = if !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            Err(HexDraftError::InvalidDigit)
        } else if digits.len() != 6 {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorEditorFocusTarget {
    Mode,
    SharedWell,
    SharedHex,
    LightWell,
    LightHex,
    DarkWell,
    DarkHex,
    NextGroup,
}

const SHARED_TAB_ORDER: &[ColorEditorFocusTarget] = &[
    ColorEditorFocusTarget::Mode,
    ColorEditorFocusTarget::SharedWell,
    ColorEditorFocusTarget::SharedHex,
    ColorEditorFocusTarget::NextGroup,
];

const APPEARANCE_TAB_ORDER: &[ColorEditorFocusTarget] = &[
    ColorEditorFocusTarget::Mode,
    ColorEditorFocusTarget::LightWell,
    ColorEditorFocusTarget::LightHex,
    ColorEditorFocusTarget::DarkWell,
    ColorEditorFocusTarget::DarkHex,
    ColorEditorFocusTarget::NextGroup,
];

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

    pub const fn tab_order(&self) -> &'static [ColorEditorFocusTarget] {
        if self.variants_enabled {
            APPEARANCE_TAB_ORDER
        } else {
            SHARED_TAB_ORDER
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
