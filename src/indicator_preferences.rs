#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrgbColor([u8; 3]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidHexColor;

impl SrgbColor {
    pub fn parse_hex(input: &str) -> Result<Self, InvalidHexColor> {
        let hex = input.strip_prefix('#').unwrap_or(input);
        if hex.len() != 6 {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LabelPreferences {
    pub visible: bool,
    pub color_mode: LabelColorMode,
    pub fixed: FixedColorPreferences,
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
        match value {
            1..=60 => Ok(Self(value)),
            _ => Err(InvalidMetricsRefreshInterval),
        }
    }
}

impl MetricsRefreshInterval {
    pub const fn seconds(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorPreferences {
    pub cpu_color: MetricColorPreferences,
    pub ram_color: MetricColorPreferences,
    pub labels: LabelPreferences,
    pub typography: TypographyPreferences,
    pub refresh_interval: MetricsRefreshInterval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorPreferenceGroup {
    CpuAndRam,
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
            labels: LabelPreferences {
                visible: true,
                color_mode: LabelColorMode::Neutral,
                fixed: FixedColorPreferences {
                    shared: SrgbColor([0x8E, 0x8E, 0x93]),
                    use_appearance_variants: false,
                    variants: None,
                },
            },
            typography: TypographyPreferences {
                family: FontFamilyPreference::SystemMonospaced,
                size: FontSize(12),
                weight: FontWeight::Medium,
            },
            refresh_interval: MetricsRefreshInterval(2),
        }
    }
}
