#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrgbColor([u8; 3]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidHexColor;

impl SrgbColor {
    pub fn parse_hex(input: &str) -> Result<Self, InvalidHexColor> {
        let hex = input.strip_prefix('#').unwrap_or(input);
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(InvalidHexColor);
        }

        let mut components = [0; 3];
        for (index, component) in components.iter_mut().enumerate() {
            let start = index * 2;
            *component =
                u8::from_str_radix(&hex[start..start + 2], 16).map_err(|_| InvalidHexColor)?;
        }

        Ok(Self(components))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0[0], self.0[1], self.0[2])
    }

    pub const fn components(self) -> [u8; 3] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorAppearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricKind {
    Cpu,
    Ram,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricIdentifierMode {
    Text,
    SystemSymbol,
    Png,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemSymbolName(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSystemSymbolName;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CuratedSystemSymbol {
    name: &'static str,
    label_pt_br: &'static str,
    introduced_year: u16,
}

pub const MACOS_14_SF_SYMBOL_CATALOG_YEAR: u16 = 2023;

impl CuratedSystemSymbol {
    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn label_pt_br(self) -> &'static str {
        self.label_pt_br
    }

    pub const fn introduced_year(self) -> u16 {
        self.introduced_year
    }
}

impl SystemSymbolName {
    // Snapshot verified from Apple's CoreGlyphs `name_availability.plist` on
    // 2026-08-13. The 2023 catalog is the one shipped with macOS 14 Sonoma.
    const CURATED_OPTIONS: [CuratedSystemSymbol; 6] = [
        CuratedSystemSymbol {
            name: "cpu",
            label_pt_br: "Processador",
            introduced_year: 2020,
        },
        CuratedSystemSymbol {
            name: "memorychip",
            label_pt_br: "Chip de memória",
            introduced_year: 2020,
        },
        CuratedSystemSymbol {
            name: "gauge.with.dots.needle.33percent",
            label_pt_br: "Medidor",
            introduced_year: 2023,
        },
        CuratedSystemSymbol {
            name: "waveform.path.ecg",
            label_pt_br: "Batimento",
            introduced_year: 2019,
        },
        CuratedSystemSymbol {
            name: "chart.bar.fill",
            label_pt_br: "Gráfico de barras",
            introduced_year: 2019,
        },
        CuratedSystemSymbol {
            name: "bolt.fill",
            label_pt_br: "Energia",
            introduced_year: 2019,
        },
    ];
    const CURATED_NAMES: [&'static str; 6] = [
        "cpu",
        "memorychip",
        "gauge.with.dots.needle.33percent",
        "waveform.path.ecg",
        "chart.bar.fill",
        "bolt.fill",
    ];

    pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidSystemSymbolName> {
        let value = value.as_ref().trim();
        if !Self::CURATED_NAMES.contains(&value) {
            return Err(InvalidSystemSymbolName);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub const fn curated_names() -> &'static [&'static str] {
        &Self::CURATED_NAMES
    }

    pub const fn curated_options() -> &'static [CuratedSystemSymbol] {
        &Self::CURATED_OPTIONS
    }

    pub fn label_pt_br(&self) -> &'static str {
        Self::CURATED_OPTIONS
            .iter()
            .find(|option| option.name == self.0)
            .map_or("Ícone do macOS", |option| option.label_pt_br)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PngIconMetadata {
    source_name: String,
    width: u32,
    height: u32,
    byte_length: u64,
    content_fingerprint: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPngIconMetadata;

impl PngIconMetadata {
    pub const MAX_DIMENSION: u32 = 24;
    pub const MAX_BYTE_LENGTH: u64 = 256 * 1024;

    pub fn new(
        source_name: impl Into<String>,
        width: u32,
        height: u32,
        byte_length: u64,
    ) -> Result<Self, InvalidPngIconMetadata> {
        Self::with_content_fingerprint(source_name, width, height, byte_length, 0)
    }

    pub fn with_content_fingerprint(
        source_name: impl Into<String>,
        width: u32,
        height: u32,
        byte_length: u64,
        content_fingerprint: u64,
    ) -> Result<Self, InvalidPngIconMetadata> {
        let source_name = source_name.into();
        let trimmed = source_name.trim();
        let has_png_extension = trimmed
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("png"));
        let is_plain_name = !trimmed.contains(['/', '\\'])
            && trimmed.chars().count() <= 128
            && !trimmed.chars().any(char::is_control);
        if trimmed.is_empty()
            || !has_png_extension
            || !is_plain_name
            || width == 0
            || height == 0
            || width > Self::MAX_DIMENSION
            || height > Self::MAX_DIMENSION
            || byte_length == 0
            || byte_length > Self::MAX_BYTE_LENGTH
        {
            return Err(InvalidPngIconMetadata);
        }
        Ok(Self {
            source_name: trimmed.to_owned(),
            width,
            height,
            byte_length,
            content_fingerprint,
        })
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub const fn content_fingerprint(&self) -> u64 {
        self.content_fingerprint
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricIdentifierPreferences {
    pub mode: MetricIdentifierMode,
    pub system_symbol: SystemSymbolName,
    pub png: Option<PngIconMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentifierPreferences {
    pub cpu: MetricIdentifierPreferences,
    pub ram: MetricIdentifierPreferences,
}

impl Default for IdentifierPreferences {
    fn default() -> Self {
        Self {
            cpu: MetricIdentifierPreferences {
                mode: MetricIdentifierMode::Text,
                system_symbol: SystemSymbolName("cpu".to_owned()),
                png: None,
            },
            ram: MetricIdentifierPreferences {
                mode: MetricIdentifierMode::Text,
                system_symbol: SystemSymbolName("memorychip".to_owned()),
                png: None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricColorMode {
    Dynamic,
    Fixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppearanceColors {
    pub light: SrgbColor,
    pub dark: SrgbColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedColorPreferences {
    pub shared: SrgbColor,
    pub use_appearance_variants: bool,
    pub variants: Option<AppearanceColors>,
}

impl FixedColorPreferences {
    pub fn set_variants_enabled(&mut self, enabled: bool) {
        if enabled && self.variants.is_none() {
            self.variants = Some(AppearanceColors {
                light: self.shared,
                dark: self.shared,
            });
        }
        self.use_appearance_variants = enabled;
    }

    pub fn color_for(self, appearance: IndicatorAppearance) -> SrgbColor {
        if self.use_appearance_variants {
            if let Some(variants) = self.variants {
                return match appearance {
                    IndicatorAppearance::Light => variants.light,
                    IndicatorAppearance::Dark => variants.dark,
                };
            }
        }

        self.shared
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricColorPreferences {
    pub mode: MetricColorMode,
    pub fixed: FixedColorPreferences,
}

impl MetricColorPreferences {
    pub const fn is_dynamic(self) -> bool {
        matches!(self.mode, MetricColorMode::Dynamic)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelColorMode {
    Neutral,
    MatchMetric,
    Fixed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorLabel(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidIndicatorLabel;

impl IndicatorLabel {
    pub const MAX_CHARACTERS: usize = 10;

    pub fn new(value: impl Into<String>) -> Result<Self, InvalidIndicatorLabel> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() || value.chars().count() > Self::MAX_CHARACTERS {
            return Err(InvalidIndicatorLabel);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LabelSpacing(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidLabelSpacing;

impl TryFrom<u8> for LabelSpacing {
    type Error = InvalidLabelSpacing;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0..=10 => Ok(Self(value)),
            _ => Err(InvalidLabelSpacing),
        }
    }
}

impl LabelSpacing {
    pub const fn level(self) -> u8 {
        self.0
    }
}

impl Default for LabelSpacing {
    fn default() -> Self {
        Self(10)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabelPreferences {
    pub visible: bool,
    pub color_mode: LabelColorMode,
    pub fixed: FixedColorPreferences,
    pub cpu: IndicatorLabel,
    pub ram: IndicatorLabel,
    pub spacing: LabelSpacing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontFamilyPreference {
    SystemMonospaced,
    Named(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidFontFamily;

impl FontFamilyPreference {
    pub fn named(value: impl Into<String>) -> Result<Self, InvalidFontFamily> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(InvalidFontFamily);
        }

        Ok(Self::Named(trimmed.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontSize(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidFontSize;

impl TryFrom<u8> for FontSize {
    type Error = InvalidFontSize;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            9..=14 => Ok(Self(value)),
            _ => Err(InvalidFontSize),
        }
    }
}

impl FontSize {
    pub const fn points(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontWeight {
    Regular,
    Medium,
    Bold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypographyPreferences {
    pub family: FontFamilyPreference,
    pub size: FontSize,
    pub weight: FontWeight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricsRefreshInterval(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidMetricsRefreshInterval;

impl TryFrom<u8> for MetricsRefreshInterval {
    type Error = InvalidMetricsRefreshInterval;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (Self::MIN_SECONDS..=Self::MAX_SECONDS).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidMetricsRefreshInterval)
        }
    }
}

impl MetricsRefreshInterval {
    pub const MIN_SECONDS: u8 = 1;
    pub const MAX_SECONDS: u8 = 60;

    pub const fn seconds(self) -> u8 {
        self.0
    }
}

impl Default for MetricsRefreshInterval {
    fn default() -> Self {
        Self(2)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorPreferences {
    pub cpu_color: MetricColorPreferences,
    pub ram_color: MetricColorPreferences,
    pub identifiers: IdentifierPreferences,
    pub labels: LabelPreferences,
    pub typography: TypographyPreferences,
    pub refresh_interval: MetricsRefreshInterval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorPreferenceGroup {
    CpuAndRam,
    Identifiers,
    Labels,
    Typography,
    RefreshInterval,
}

impl IndicatorPreferences {
    pub fn reset(&mut self, group: IndicatorPreferenceGroup) {
        let defaults = Self::default();
        match group {
            IndicatorPreferenceGroup::CpuAndRam => {
                self.cpu_color = defaults.cpu_color;
                self.ram_color = defaults.ram_color;
            }
            IndicatorPreferenceGroup::Identifiers => self.identifiers = defaults.identifiers,
            IndicatorPreferenceGroup::Labels => self.labels = defaults.labels,
            IndicatorPreferenceGroup::Typography => self.typography = defaults.typography,
            IndicatorPreferenceGroup::RefreshInterval => {
                self.refresh_interval = defaults.refresh_interval;
            }
        }
    }
}

impl Default for IndicatorPreferences {
    fn default() -> Self {
        Self {
            cpu_color: MetricColorPreferences {
                mode: MetricColorMode::Dynamic,
                fixed: FixedColorPreferences {
                    shared: SrgbColor([0x34, 0xC7, 0x59]),
                    use_appearance_variants: false,
                    variants: None,
                },
            },
            ram_color: MetricColorPreferences {
                mode: MetricColorMode::Dynamic,
                fixed: FixedColorPreferences {
                    shared: SrgbColor([0x0A, 0x84, 0xFF]),
                    use_appearance_variants: false,
                    variants: None,
                },
            },
            identifiers: IdentifierPreferences::default(),
            labels: LabelPreferences {
                visible: true,
                color_mode: LabelColorMode::Neutral,
                fixed: FixedColorPreferences {
                    shared: SrgbColor([0x8E, 0x8E, 0x93]),
                    use_appearance_variants: false,
                    variants: None,
                },
                cpu: IndicatorLabel("C".to_owned()),
                ram: IndicatorLabel("R".to_owned()),
                spacing: LabelSpacing::default(),
            },
            typography: TypographyPreferences {
                family: FontFamilyPreference::SystemMonospaced,
                size: FontSize(12),
                weight: FontWeight::Medium,
            },
            refresh_interval: MetricsRefreshInterval::default(),
        }
    }
}
