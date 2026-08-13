use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::{Preferences, WarningThreshold};
use crate::indicator_preferences::{
    AppearanceColors, FixedColorPreferences, FontFamilyPreference, FontSize, FontWeight,
    IdentifierPreferences, IndicatorLabel, IndicatorPreferences, LabelColorMode, LabelPreferences,
    LabelSpacing, MetricColorMode, MetricColorPreferences, MetricIdentifierMode,
    MetricIdentifierPreferences, MetricsRefreshInterval, PngIconMetadata, SrgbColor,
    SystemSymbolName, TypographyPreferences,
};

const CURRENT_VERSION: u8 = 2;

#[derive(Clone, Debug)]
pub struct PreferencesStore {
    path: PathBuf,
}

impl PreferencesStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_current_user() -> io::Result<Self> {
        let override_path = std::env::var_os("STATLET_PREFERENCES_PATH").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::for_user_locations(override_path, home)
    }

    fn for_user_locations(
        override_path: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> io::Result<Self> {
        if let Some(path) = override_path {
            return Ok(Self::new(path));
        }
        let home =
            home.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unavailable"))?;
        Ok(Self::new(home.join(
            "Library/Application Support/Statlet/preferences.json",
        )))
    }

    pub fn load(&self) -> Preferences {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| decode(&bytes))
            .unwrap_or_default()
    }

    pub fn save(&self, preferences: Preferences) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "preferences path has no parent",
            )
        })?;
        fs::create_dir_all(parent)?;

        let temporary_path = temporary_path(&self.path);
        let result = self.write_and_replace(parent, &temporary_path, preferences);
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    fn write_and_replace(
        &self,
        parent: &Path,
        temporary_path: &Path,
        preferences: Preferences,
    ) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary_path)?;
        let stored = StoredPreferencesV2::from(preferences);
        serde_json::to_writer_pretty(&mut file, &stored).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(temporary_path, &self.path)?;
        File::open(parent)?.sync_all()
    }
}

#[cfg(test)]
mod location_tests {
    use super::PreferencesStore;
    use std::path::PathBuf;

    #[test]
    fn explicit_preferences_path_does_not_depend_on_home() {
        let expected = PathBuf::from("/tmp/statlet-test/preferences.json");
        let store = PreferencesStore::for_user_locations(Some(expected.clone()), None).unwrap();

        assert_eq!(store.path, expected);
    }

    #[test]
    fn current_user_path_remains_the_default_without_an_override() {
        let home = PathBuf::from("/Users/example");
        let store = PreferencesStore::for_user_locations(None, Some(home)).unwrap();

        assert_eq!(
            store.path,
            PathBuf::from("/Users/example/Library/Application Support/Statlet/preferences.json")
        );
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("preferences.json");
    destination.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()))
}

#[derive(Deserialize)]
struct StoredVersion {
    version: u8,
}

fn decode(bytes: &[u8]) -> Option<Preferences> {
    match serde_json::from_slice::<StoredVersion>(bytes).ok()?.version {
        1 => serde_json::from_slice::<StoredPreferencesV1>(bytes)
            .ok()?
            .into_preferences(),
        2 => serde_json::from_slice::<StoredPreferencesV2>(bytes)
            .ok()?
            .into_preferences(),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreferencesV1 {
    version: u8,
    mole_integration_enabled: bool,
    warning_threshold: u8,
}

impl StoredPreferencesV1 {
    fn into_preferences(self) -> Option<Preferences> {
        if self.version != 1 {
            return None;
        }

        Some(Preferences {
            mole_integration_enabled: self.mole_integration_enabled,
            warning_threshold: WarningThreshold::try_from(self.warning_threshold).ok()?,
            ..Preferences::default()
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreferencesV2 {
    version: u8,
    mole_integration_enabled: bool,
    warning_threshold: u8,
    indicator: StoredIndicatorPreferences,
}

impl From<Preferences> for StoredPreferencesV2 {
    fn from(preferences: Preferences) -> Self {
        Self {
            version: CURRENT_VERSION,
            mole_integration_enabled: preferences.mole_integration_enabled,
            warning_threshold: preferences.warning_threshold.get(),
            indicator: preferences.indicator.into(),
        }
    }
}

impl StoredPreferencesV2 {
    fn into_preferences(self) -> Option<Preferences> {
        if self.version != CURRENT_VERSION {
            return None;
        }

        Some(Preferences {
            mole_integration_enabled: self.mole_integration_enabled,
            warning_threshold: WarningThreshold::try_from(self.warning_threshold).ok()?,
            indicator: self.indicator.into_preferences()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredIndicatorPreferences {
    cpu_color: StoredMetricColorPreferences,
    ram_color: StoredMetricColorPreferences,
    #[serde(default)]
    identifiers: StoredIdentifierPreferences,
    labels: StoredLabelPreferences,
    typography: StoredTypographyPreferences,
    refresh_interval: u8,
}

impl From<IndicatorPreferences> for StoredIndicatorPreferences {
    fn from(preferences: IndicatorPreferences) -> Self {
        Self {
            cpu_color: preferences.cpu_color.into(),
            ram_color: preferences.ram_color.into(),
            identifiers: preferences.identifiers.into(),
            labels: preferences.labels.into(),
            typography: preferences.typography.into(),
            refresh_interval: preferences.refresh_interval.seconds(),
        }
    }
}

impl StoredIndicatorPreferences {
    fn into_preferences(self) -> Option<IndicatorPreferences> {
        Some(IndicatorPreferences {
            cpu_color: self.cpu_color.into_preferences()?,
            ram_color: self.ram_color.into_preferences()?,
            identifiers: self.identifiers.into_preferences()?,
            labels: self.labels.into_preferences()?,
            typography: self.typography.into_preferences()?,
            refresh_interval: MetricsRefreshInterval::try_from(self.refresh_interval).ok()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredIdentifierPreferences {
    cpu: StoredMetricIdentifierPreferences,
    ram: StoredMetricIdentifierPreferences,
}

impl Default for StoredIdentifierPreferences {
    fn default() -> Self {
        IndicatorPreferences::default().identifiers.into()
    }
}

impl From<IdentifierPreferences> for StoredIdentifierPreferences {
    fn from(preferences: IdentifierPreferences) -> Self {
        Self {
            cpu: preferences.cpu.into(),
            ram: preferences.ram.into(),
        }
    }
}

impl StoredIdentifierPreferences {
    fn into_preferences(self) -> Option<IdentifierPreferences> {
        Some(IdentifierPreferences {
            cpu: self.cpu.into_preferences()?,
            ram: self.ram.into_preferences()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredMetricIdentifierPreferences {
    mode: StoredMetricIdentifierMode,
    system_symbol: String,
    png: Option<StoredPngIconMetadata>,
}

impl From<MetricIdentifierPreferences> for StoredMetricIdentifierPreferences {
    fn from(preferences: MetricIdentifierPreferences) -> Self {
        Self {
            mode: preferences.mode.into(),
            system_symbol: preferences.system_symbol.as_str().to_owned(),
            png: preferences.png.map(Into::into),
        }
    }
}

impl StoredMetricIdentifierPreferences {
    fn into_preferences(self) -> Option<MetricIdentifierPreferences> {
        Some(MetricIdentifierPreferences {
            mode: self.mode.into(),
            system_symbol: SystemSymbolName::new(self.system_symbol).ok()?,
            png: self
                .png
                .map(StoredPngIconMetadata::into_preferences)
                .transpose()
                .ok()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StoredMetricIdentifierMode {
    Text,
    SystemSymbol,
    Png,
}

impl From<MetricIdentifierMode> for StoredMetricIdentifierMode {
    fn from(mode: MetricIdentifierMode) -> Self {
        match mode {
            MetricIdentifierMode::Text => Self::Text,
            MetricIdentifierMode::SystemSymbol => Self::SystemSymbol,
            MetricIdentifierMode::Png => Self::Png,
        }
    }
}

impl From<StoredMetricIdentifierMode> for MetricIdentifierMode {
    fn from(mode: StoredMetricIdentifierMode) -> Self {
        match mode {
            StoredMetricIdentifierMode::Text => Self::Text,
            StoredMetricIdentifierMode::SystemSymbol => Self::SystemSymbol,
            StoredMetricIdentifierMode::Png => Self::Png,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredPngIconMetadata {
    source_name: String,
    width: u32,
    height: u32,
    byte_length: u64,
    #[serde(default)]
    content_fingerprint: u64,
}

impl From<PngIconMetadata> for StoredPngIconMetadata {
    fn from(metadata: PngIconMetadata) -> Self {
        Self {
            source_name: metadata.source_name().to_owned(),
            width: metadata.width(),
            height: metadata.height(),
            byte_length: metadata.byte_length(),
            content_fingerprint: metadata.content_fingerprint(),
        }
    }
}

impl StoredPngIconMetadata {
    fn into_preferences(
        self,
    ) -> Result<PngIconMetadata, crate::indicator_preferences::InvalidPngIconMetadata> {
        PngIconMetadata::with_content_fingerprint(
            self.source_name,
            self.width,
            self.height,
            self.byte_length,
            self.content_fingerprint,
        )
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredMetricColorPreferences {
    mode: StoredMetricColorMode,
    fixed: StoredFixedColorPreferences,
}

impl From<MetricColorPreferences> for StoredMetricColorPreferences {
    fn from(preferences: MetricColorPreferences) -> Self {
        Self {
            mode: preferences.mode.into(),
            fixed: preferences.fixed.into(),
        }
    }
}

impl StoredMetricColorPreferences {
    fn into_preferences(self) -> Option<MetricColorPreferences> {
        Some(MetricColorPreferences {
            mode: self.mode.into(),
            fixed: self.fixed.into_preferences()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StoredMetricColorMode {
    Dynamic,
    Fixed,
}

impl From<MetricColorMode> for StoredMetricColorMode {
    fn from(mode: MetricColorMode) -> Self {
        match mode {
            MetricColorMode::Dynamic => Self::Dynamic,
            MetricColorMode::Fixed => Self::Fixed,
        }
    }
}

impl From<StoredMetricColorMode> for MetricColorMode {
    fn from(mode: StoredMetricColorMode) -> Self {
        match mode {
            StoredMetricColorMode::Dynamic => Self::Dynamic,
            StoredMetricColorMode::Fixed => Self::Fixed,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredFixedColorPreferences {
    shared: String,
    use_appearance_variants: bool,
    variants: Option<StoredAppearanceColors>,
}

impl From<FixedColorPreferences> for StoredFixedColorPreferences {
    fn from(preferences: FixedColorPreferences) -> Self {
        Self {
            shared: preferences.shared.to_hex(),
            use_appearance_variants: preferences.use_appearance_variants,
            variants: preferences.variants.map(Into::into),
        }
    }
}

impl StoredFixedColorPreferences {
    fn into_preferences(self) -> Option<FixedColorPreferences> {
        Some(FixedColorPreferences {
            shared: SrgbColor::parse_hex(&self.shared).ok()?,
            use_appearance_variants: self.use_appearance_variants,
            variants: match self.variants {
                Some(colors) => Some(colors.into_preferences()?),
                None => None,
            },
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredAppearanceColors {
    light: String,
    dark: String,
}

impl From<AppearanceColors> for StoredAppearanceColors {
    fn from(colors: AppearanceColors) -> Self {
        Self {
            light: colors.light.to_hex(),
            dark: colors.dark.to_hex(),
        }
    }
}

impl StoredAppearanceColors {
    fn into_preferences(self) -> Option<AppearanceColors> {
        Some(AppearanceColors {
            light: SrgbColor::parse_hex(&self.light).ok()?,
            dark: SrgbColor::parse_hex(&self.dark).ok()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredLabelPreferences {
    visible: bool,
    color_mode: StoredLabelColorMode,
    fixed: StoredFixedColorPreferences,
    #[serde(default)]
    cpu: Option<String>,
    #[serde(default)]
    ram: Option<String>,
    #[serde(default)]
    spacing: Option<u8>,
}

impl From<LabelPreferences> for StoredLabelPreferences {
    fn from(preferences: LabelPreferences) -> Self {
        Self {
            visible: preferences.visible,
            color_mode: preferences.color_mode.into(),
            fixed: preferences.fixed.into(),
            cpu: Some(preferences.cpu.as_str().to_owned()),
            ram: Some(preferences.ram.as_str().to_owned()),
            spacing: Some(preferences.spacing.spaces() as u8),
        }
    }
}

impl StoredLabelPreferences {
    fn into_preferences(self) -> Option<LabelPreferences> {
        let defaults = IndicatorPreferences::default().labels;
        Some(LabelPreferences {
            visible: self.visible,
            color_mode: self.color_mode.into(),
            fixed: self.fixed.into_preferences()?,
            cpu: self
                .cpu
                .map(IndicatorLabel::new)
                .transpose()
                .ok()?
                .unwrap_or(defaults.cpu),
            ram: self
                .ram
                .map(IndicatorLabel::new)
                .transpose()
                .ok()?
                .unwrap_or(defaults.ram),
            spacing: self
                .spacing
                .map(LabelSpacing::try_from)
                .transpose()
                .ok()?
                .unwrap_or(defaults.spacing),
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StoredLabelColorMode {
    Neutral,
    MatchMetric,
    Fixed,
}

impl From<LabelColorMode> for StoredLabelColorMode {
    fn from(mode: LabelColorMode) -> Self {
        match mode {
            LabelColorMode::Neutral => Self::Neutral,
            LabelColorMode::MatchMetric => Self::MatchMetric,
            LabelColorMode::Fixed => Self::Fixed,
        }
    }
}

impl From<StoredLabelColorMode> for LabelColorMode {
    fn from(mode: StoredLabelColorMode) -> Self {
        match mode {
            StoredLabelColorMode::Neutral => Self::Neutral,
            StoredLabelColorMode::MatchMetric => Self::MatchMetric,
            StoredLabelColorMode::Fixed => Self::Fixed,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredTypographyPreferences {
    family: StoredFontFamily,
    size: u8,
    weight: StoredFontWeight,
}

impl From<TypographyPreferences> for StoredTypographyPreferences {
    fn from(preferences: TypographyPreferences) -> Self {
        Self {
            family: preferences.family.into(),
            size: preferences.size.points(),
            weight: preferences.weight.into(),
        }
    }
}

impl StoredTypographyPreferences {
    fn into_preferences(self) -> Option<TypographyPreferences> {
        Some(TypographyPreferences {
            family: self.family.into_preferences()?,
            size: FontSize::try_from(self.size).ok()?,
            weight: self.weight.into(),
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StoredFontFamily {
    SystemMonospaced,
    Named(String),
}

impl From<FontFamilyPreference> for StoredFontFamily {
    fn from(family: FontFamilyPreference) -> Self {
        match family {
            FontFamilyPreference::SystemMonospaced => Self::SystemMonospaced,
            FontFamilyPreference::Named(name) => Self::Named(name),
        }
    }
}

impl StoredFontFamily {
    fn into_preferences(self) -> Option<FontFamilyPreference> {
        match self {
            Self::SystemMonospaced => Some(FontFamilyPreference::SystemMonospaced),
            Self::Named(name) => FontFamilyPreference::named(name).ok(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StoredFontWeight {
    Regular,
    Medium,
    Bold,
}

impl From<FontWeight> for StoredFontWeight {
    fn from(weight: FontWeight) -> Self {
        match weight {
            FontWeight::Regular => Self::Regular,
            FontWeight::Medium => Self::Medium,
            FontWeight::Bold => Self::Bold,
        }
    }
}

impl From<StoredFontWeight> for FontWeight {
    fn from(weight: StoredFontWeight) -> Self {
        match weight {
            StoredFontWeight::Regular => Self::Regular,
            StoredFontWeight::Medium => Self::Medium,
            StoredFontWeight::Bold => Self::Bold,
        }
    }
}
