use crate::core::{AppState, Preferences, PreferencesSaveStatus};
use crate::indicator_preferences::{
    AppearanceColors, FixedColorPreferences, MetricIdentifierMode, MetricIdentifierPreferences,
    MetricKind, MetricsRefreshInterval, SrgbColor,
};

mod layout;

pub use layout::{
    preserve_scroll_origin_from_top, ControlSlot, IndicatorControlsLayout,
    IndicatorControlsVisibility, MessageLayout, RowSlot, VerticalSlot,
};

const SYSTEM_MONOSPACED_LABEL: &str = "System Monospaced";
const PREFERENCES_SAVE_ERROR: &str = "Não foi possível salvar as preferências.";
const GENERAL_TITLE: &str = "Geral";
const SHOW_IN_MENU_BAR_LABEL: &str = "Mostrar o Statlet na barra de menus";
const SHOW_IN_MENU_BAR_IDENTIFIER: &str = "general.show-in-menu-bar";
const MENU_BAR_RECOVERY_HELP: &str =
    "Se ocultar o Statlet, abra-o pelo Finder ou Spotlight para voltar às Preferências.";
pub const GENERAL_RECOVERY_MAX_LINES: usize = 2;
pub const GENERAL_RECOVERY_LAYOUT_WIDTH: f64 = 400.0;
pub const GENERAL_RECOVERY_LAYOUT_HEIGHT: f64 = 44.0;
pub const fn general_recovery_layout() -> (f64, f64, usize) {
    (
        GENERAL_RECOVERY_LAYOUT_WIDTH,
        GENERAL_RECOVERY_LAYOUT_HEIGHT,
        GENERAL_RECOVERY_MAX_LINES,
    )
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreferencesNavigationArea {
    #[default]
    General,
    Colors,
    Labels,
    Typography,
    Refresh,
    DiskAndMole,
}

impl PreferencesNavigationArea {
    pub const fn from_sidebar_row(row: isize) -> Option<Self> {
        match row {
            0 => Some(Self::General),
            1 => Some(Self::Colors),
            2 => Some(Self::Labels),
            3 => Some(Self::Typography),
            4 => Some(Self::Refresh),
            5 => Some(Self::DiskAndMole),
            _ => None,
        }
    }

    pub const fn sidebar_label(self) -> &'static str {
        match self {
            Self::General => "Geral",
            Self::Colors => "Cores",
            Self::Labels => "Rótulos",
            Self::Typography => "Tipografia",
            Self::Refresh => "Atualização",
            Self::DiskAndMole => "Disco e Mole",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeneralPreferencesPresentation {
    show_in_menu_bar: bool,
}

impl GeneralPreferencesPresentation {
    pub const fn from_preferences(preferences: &Preferences) -> Self {
        Self {
            show_in_menu_bar: preferences.show_in_menu_bar,
        }
    }

    pub const fn show_in_menu_bar(self) -> bool {
        self.show_in_menu_bar
    }

    pub const fn title(self) -> &'static str {
        GENERAL_TITLE
    }

    pub const fn toggle_label(self) -> &'static str {
        SHOW_IN_MENU_BAR_LABEL
    }

    pub const fn toggle_identifier(self) -> &'static str {
        SHOW_IN_MENU_BAR_IDENTIFIER
    }

    pub const fn recovery_help(self) -> &'static str {
        MENU_BAR_RECOVERY_HELP
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreferencesArea {
    #[default]
    Colors,
    Labels,
    Typography,
    Refresh,
    DiskAndMole,
}

impl PreferencesArea {
    pub const fn from_sidebar_row(row: isize) -> Option<Self> {
        match row {
            0 => Some(Self::Colors),
            1 => Some(Self::Labels),
            2 => Some(Self::Typography),
            3 => Some(Self::Refresh),
            4 => Some(Self::DiskAndMole),
            _ => None,
        }
    }

    pub const fn sidebar_label(self) -> &'static str {
        match self {
            Self::Colors => "Cores",
            Self::Labels => "Rótulos",
            Self::Typography => "Tipografia",
            Self::Refresh => "Atualização",
            Self::DiskAndMole => "Disco e Mole",
        }
    }

    pub const fn indicator_index(self) -> Option<usize> {
        match self {
            Self::Colors => Some(0),
            Self::Labels => Some(1),
            Self::Typography => Some(2),
            Self::Refresh => Some(3),
            Self::DiskAndMole => None,
        }
    }

    pub const fn is_indicator(self) -> bool {
        self.indicator_index().is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferencesShellFocusTarget {
    ResetIndicator,
    RetrySave,
    Sidebar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreferencesShellPresentation {
    area: PreferencesArea,
    can_undo_indicator_reset: bool,
    save_status: PreferencesSaveStatus,
}

impl PreferencesShellPresentation {
    pub const fn new(
        area: PreferencesArea,
        can_undo_indicator_reset: bool,
        save_status: PreferencesSaveStatus,
    ) -> Self {
        Self {
            area,
            can_undo_indicator_reset,
            save_status,
        }
    }

    pub const fn indicator_reset_visible(self) -> bool {
        self.area.is_indicator()
    }

    pub const fn undo_visible(self) -> bool {
        self.indicator_reset_visible() && self.can_undo_indicator_reset
    }

    pub const fn retry_visible(self) -> bool {
        matches!(self.save_status, PreferencesSaveStatus::Failed)
    }

    pub const fn save_error(self) -> Option<&'static str> {
        if self.retry_visible() {
            Some(PREFERENCES_SAVE_ERROR)
        } else {
            None
        }
    }

    pub const fn focus_target_after_area_controls(self) -> PreferencesShellFocusTarget {
        if self.indicator_reset_visible() {
            PreferencesShellFocusTarget::ResetIndicator
        } else if self.retry_visible() {
            PreferencesShellFocusTarget::RetrySave
        } else {
            PreferencesShellFocusTarget::Sidebar
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreferencesNavigationPolicy {
    area_changed: bool,
}

impl PreferencesNavigationPolicy {
    pub fn between<T: PartialEq>(current: T, selected: T) -> Self {
        Self {
            area_changed: current != selected,
        }
    }

    pub fn scroll_origin_y(
        self,
        origin_y: f64,
        viewport_height: f64,
        old_document_height: f64,
        new_document_height: f64,
    ) -> f64 {
        if self.area_changed {
            (new_document_height - viewport_height).max(0.0)
        } else {
            preserve_scroll_origin_from_top(
                origin_y,
                viewport_height,
                old_document_height,
                new_document_height,
            )
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreferencesControlsPresentation {
    preferences: Preferences,
    can_undo_indicator_reset: bool,
    preferences_save_status: PreferencesSaveStatus,
    indicator_icon_errors: [Option<String>; 2],
    indicator_icon_pending: [bool; 2],
}

impl PreferencesControlsPresentation {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            preferences: state.preferences.clone(),
            can_undo_indicator_reset: state.can_undo_indicator_reset,
            preferences_save_status: state.preferences_save_status,
            indicator_icon_errors: [
                state
                    .indicator_icon_error(MetricKind::Cpu)
                    .map(str::to_owned),
                state
                    .indicator_icon_error(MetricKind::Ram)
                    .map(str::to_owned),
            ],
            indicator_icon_pending: [
                state.indicator_icon_pending(MetricKind::Cpu),
                state.indicator_icon_pending(MetricKind::Ram),
            ],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentifierDetailPresentation {
    Hidden,
    SystemSymbol {
        selected_name: String,
    },
    Png {
        source_name: Option<String>,
        can_remove: bool,
    },
}

const LATENT_TEXT_LABEL_HELP: &str = "Preservado para o modo Texto.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelEditingFocusTarget {
    CpuLabel,
    RamLabel,
    Spacing,
    LabelColorMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierEditingFocusTarget {
    CpuMode,
    CpuSymbol,
    CpuChoosePng,
    CpuRemovePng,
    RamMode,
    RamSymbol,
    RamChoosePng,
    RamRemovePng,
    SystemSymbolSize,
    ResetIdentifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentifierEditingPresentation {
    cpu_mode: MetricIdentifierMode,
    ram_mode: MetricIdentifierMode,
}

impl IdentifierEditingPresentation {
    pub const fn new(cpu_mode: MetricIdentifierMode, ram_mode: MetricIdentifierMode) -> Self {
        Self { cpu_mode, ram_mode }
    }

    pub const fn system_symbol_size_enabled(self) -> bool {
        matches!(self.cpu_mode, MetricIdentifierMode::SystemSymbol)
            || matches!(self.ram_mode, MetricIdentifierMode::SystemSymbol)
    }

    pub const fn system_symbol_size_help(self) -> &'static str {
        "Ajusta o tamanho compartilhado dos ícones do macOS de CPU e RAM."
    }

    pub fn focus_order(
        self,
        cpu_png_available: bool,
        ram_png_available: bool,
    ) -> Vec<IdentifierEditingFocusTarget> {
        use IdentifierEditingFocusTarget::*;
        let mut order = vec![CpuMode];
        match self.cpu_mode {
            MetricIdentifierMode::Text => {}
            MetricIdentifierMode::SystemSymbol => order.push(CpuSymbol),
            MetricIdentifierMode::Png => {
                order.push(CpuChoosePng);
                if cpu_png_available {
                    order.push(CpuRemovePng);
                }
            }
        }
        order.push(RamMode);
        match self.ram_mode {
            MetricIdentifierMode::Text => {}
            MetricIdentifierMode::SystemSymbol => order.push(RamSymbol),
            MetricIdentifierMode::Png => {
                order.push(RamChoosePng);
                if ram_png_available {
                    order.push(RamRemovePng);
                }
            }
        }
        if self.system_symbol_size_enabled() {
            order.push(SystemSymbolSize);
        }
        order.push(ResetIdentifiers);
        order
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LabelEditingPresentation {
    cpu_enabled: bool,
    ram_enabled: bool,
}

impl LabelEditingPresentation {
    pub const fn new(cpu_mode: MetricIdentifierMode, ram_mode: MetricIdentifierMode) -> Self {
        Self {
            cpu_enabled: matches!(cpu_mode, MetricIdentifierMode::Text),
            ram_enabled: matches!(ram_mode, MetricIdentifierMode::Text),
        }
    }

    pub const fn cpu_enabled(self) -> bool {
        self.cpu_enabled
    }

    pub const fn ram_enabled(self) -> bool {
        self.ram_enabled
    }

    pub const fn spacing_enabled(self) -> bool {
        self.cpu_enabled || self.ram_enabled
    }

    pub const fn cpu_help(self) -> Option<&'static str> {
        if self.cpu_enabled {
            None
        } else {
            Some(LATENT_TEXT_LABEL_HELP)
        }
    }

    pub const fn ram_help(self) -> Option<&'static str> {
        if self.ram_enabled {
            None
        } else {
            Some(LATENT_TEXT_LABEL_HELP)
        }
    }

    pub const fn spacing_help(self) -> Option<&'static str> {
        if self.spacing_enabled() {
            None
        } else {
            Some(LATENT_TEXT_LABEL_HELP)
        }
    }

    pub fn focus_order(self) -> Vec<LabelEditingFocusTarget> {
        let mut order = Vec::with_capacity(4);
        if self.cpu_enabled {
            order.push(LabelEditingFocusTarget::CpuLabel);
        }
        if self.ram_enabled {
            order.push(LabelEditingFocusTarget::RamLabel);
        }
        if self.spacing_enabled() {
            order.push(LabelEditingFocusTarget::Spacing);
        }
        order.push(LabelEditingFocusTarget::LabelColorMode);
        order
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypographyWarningKind {
    FontFallback,
    Layout,
}

impl TypographyWarningKind {
    pub const fn accessibility_identifier(self) -> &'static str {
        match self {
            Self::FontFallback => "indicator.font.fallback-warning",
            Self::Layout => "indicator.font.layout-warning",
        }
    }

    pub const fn accessibility_label(self, message: Option<&str>) -> Option<&str> {
        match self {
            Self::FontFallback | Self::Layout => message,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricIdentifierControlPresentation {
    pub detail: IdentifierDetailPresentation,
    pub error: Option<String>,
    pub processing: bool,
}

impl MetricIdentifierControlPresentation {
    pub fn new(preferences: &MetricIdentifierPreferences, error: Option<&str>) -> Self {
        Self::with_processing(preferences, error, false)
    }

    pub fn with_processing(
        preferences: &MetricIdentifierPreferences,
        error: Option<&str>,
        processing: bool,
    ) -> Self {
        let detail = if processing {
            IdentifierDetailPresentation::Png {
                source_name: preferences
                    .png
                    .as_ref()
                    .map(|metadata| metadata.source_name().to_owned()),
                can_remove: false,
            }
        } else {
            match preferences.mode {
                MetricIdentifierMode::Text => IdentifierDetailPresentation::Hidden,
                MetricIdentifierMode::SystemSymbol => IdentifierDetailPresentation::SystemSymbol {
                    selected_name: preferences.system_symbol.as_str().to_owned(),
                },
                MetricIdentifierMode::Png => IdentifierDetailPresentation::Png {
                    source_name: preferences
                        .png
                        .as_ref()
                        .map(|metadata| metadata.source_name().to_owned()),
                    can_remove: preferences.png.is_some(),
                },
            }
        };
        Self {
            detail,
            error: error.map(str::to_owned),
            processing,
        }
    }
}

#[derive(Debug, Default)]
pub struct PreferencesControlsCache {
    current: Option<PreferencesControlsPresentation>,
}

impl PreferencesControlsCache {
    pub fn should_apply(&mut self, state: &AppState) -> bool {
        let next = PreferencesControlsPresentation::from_state(state);
        if self.current.as_ref() == Some(&next) {
            return false;
        }
        self.current = Some(next);
        true
    }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontPickerInteraction {
    NavigateTo(usize),
    Activate(usize),
}

impl FontPickerInteraction {
    pub const fn confirmed_row(self) -> Option<usize> {
        match self {
            Self::NavigateTo(_) => None,
            Self::Activate(row) => Some(row),
        }
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
    pub fn message(self) -> String {
        format!(
            "Digite um número inteiro de {} a {}.",
            MetricsRefreshInterval::MIN_SECONDS,
            MetricsRefreshInterval::MAX_SECONDS
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntervalFieldFormat {
    minimum: u8,
    maximum: u8,
}

impl IntervalFieldFormat {
    pub const fn seconds() -> Self {
        Self {
            minimum: MetricsRefreshInterval::MIN_SECONDS,
            maximum: MetricsRefreshInterval::MAX_SECONDS,
        }
    }

    pub const fn minimum(self) -> u8 {
        self.minimum
    }

    pub const fn maximum(self) -> u8 {
        self.maximum
    }

    pub const fn allows_floats(self) -> bool {
        false
    }

    pub const fn uses_grouping_separator(self) -> bool {
        false
    }

    pub const fn validates_partial_input(self) -> bool {
        true
    }

    pub const fn accepts_invalid_commit_for_domain_validation(self) -> bool {
        true
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
    VariantsToggle,
    NextGroup,
}

const SHARED_TAB_ORDER: &[ColorEditorFocusTarget] = &[
    ColorEditorFocusTarget::Mode,
    ColorEditorFocusTarget::SharedWell,
    ColorEditorFocusTarget::SharedHex,
    ColorEditorFocusTarget::VariantsToggle,
    ColorEditorFocusTarget::NextGroup,
];

const APPEARANCE_TAB_ORDER: &[ColorEditorFocusTarget] = &[
    ColorEditorFocusTarget::Mode,
    ColorEditorFocusTarget::LightWell,
    ColorEditorFocusTarget::LightHex,
    ColorEditorFocusTarget::DarkWell,
    ColorEditorFocusTarget::DarkHex,
    ColorEditorFocusTarget::VariantsToggle,
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
