use std::path::PathBuf;
use std::time::Duration;

use crate::disk::DiskObservation;
use crate::history::HistoryEventKind;
use crate::indicator_preferences::{
    AppearanceColors, FontFamilyPreference, FontSize, FontWeight, IdentifierPreferences,
    IndicatorAppearance, IndicatorLabel, IndicatorPreferenceGroup, IndicatorPreferences,
    LabelColorMode, LabelSpacing, MetricColorMode, MetricColorPreferences, MetricIdentifierMode,
    MetricIdentifierPreferences, MetricKind, MetricsRefreshInterval, PngIconMetadata, SrgbColor,
    SystemSymbolName,
};
use crate::mole::MoleStatus;
use crate::system_usage::SystemUsageSection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownMemoryPressure(pub i32);

impl TryFrom<i32> for MemoryPressure {
    type Error = UnknownMemoryPressure;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Normal),
            2 => Ok(Self::Warning),
            4 => Ok(Self::Critical),
            other => Err(UnknownMemoryPressure(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemSnapshot {
    pub cpu_percent: f64,
    pub ram_percent: f64,
    pub memory_pressure: MemoryPressure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricSeverity {
    Good,
    Warning,
    Critical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricContent {
    pub label: &'static str,
    pub percent: u8,
    pub severity: MetricSeverity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusContent {
    pub cpu: MetricContent,
    pub ram: MetricContent,
    pub disk_badge: Option<DiskBadge>,
    pub accessibility_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiskBadge {
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    pub status: StatusContent,
    pub preferences: Preferences,
    pub latest_disk_observation: Option<DiskObservation>,
    pub mole_status: MoleStatus,
    pub can_undo_indicator_reset: bool,
    pub preferences_save_status: PreferencesSaveStatus,
    indicator_icon_errors: [Option<String>; 2],
    indicator_icon_error_owners: [Option<IndicatorIconErrorOwner>; 2],
    indicator_icon_pending: [bool; 2],
    pub system_usage_section: SystemUsageSection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndicatorIconErrorOwner {
    Independent,
    PreferencesDurability,
}

impl AppState {
    pub fn indicator_icon_error(&self, metric: MetricKind) -> Option<&str> {
        self.indicator_icon_errors[metric_index(metric)].as_deref()
    }

    pub const fn indicator_icon_pending(&self, metric: MetricKind) -> bool {
        self.indicator_icon_pending[metric_index(metric)]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    ApplicationLaunched,
    ApplicationReopened {
        has_visible_windows: bool,
    },
    MetricsSample(SystemSnapshot),
    OpenPreferences,
    OpenHistory,
    OpenSystemUsage,
    SelectSystemUsageSection(SystemUsageSection),
    Quit,
    SetMoleIntegrationEnabled(bool),
    SetWarningThreshold(WarningThreshold),
    DiskObserved(DiskObservation),
    DiskMonitoringFailed,
    ReviewSpace,
    NotificationActivated,
    MoleStatusObserved(MoleStatus),
    OpenMoleInTerminal,
    ClearHistoryConfirmed,
    ChooseMetricPng {
        metric: MetricKind,
        source: PathBuf,
    },
    CancelMetricPngImport(MetricKind),
    MetricPngImportFinished {
        metric: MetricKind,
        result: MetricPngImportResult,
    },
    RemoveMetricPng(MetricKind),
    MetricPngRemovalFinished {
        metric: MetricKind,
        result: MetricPngRemovalResult,
    },
    MetricPngPersistenceFailed {
        metric: MetricKind,
        previous: MetricIdentifierPreferences,
        message: String,
    },
    MetricPngTransactionCleanupFailed {
        metric: MetricKind,
        message: String,
    },
    MetricPngDurabilityWarning {
        metric: MetricKind,
        message: String,
    },
    IdentifierResetPersistenceFailed {
        previous: IdentifierPreferences,
        message: String,
    },
    GlobalIndicatorResetPersistenceFailed(Box<GlobalIndicatorResetFailure>),
    GlobalIndicatorUndoPersistenceFailed(Box<GlobalIndicatorUndoFailure>),
    UpdateIndicator(IndicatorPreferenceChange),
    ResetIndicatorGroup(IndicatorPreferenceGroup),
    ResetIndicatorConfirmed,
    UndoIndicatorReset,
    PreferencesWindowClosed,
    RetrySavePreferences,
    PreferencesSaveFinished(PreferencesSaveResult),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferencesSaveStatus {
    Saved,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferencesSaveResult {
    Saved,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalIndicatorResetFailure {
    pub previous: IndicatorPreferences,
    pub replaced_undo: Option<IndicatorPreferences>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalIndicatorUndoFailure {
    pub current: IndicatorPreferences,
    pub undo: IndicatorPreferences,
    pub message: String,
    pub stage: GlobalIndicatorUndoFailureStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalIndicatorUndoFailureStage {
    AssetPreparation,
    Persistence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricPngImportResult {
    Imported(PngIconMetadata),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricPngRemovalResult {
    Removed,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricPngAssetMutation {
    Replace,
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndicatorPreferenceChange {
    SetMetricIdentifierMode {
        metric: MetricKind,
        mode: MetricIdentifierMode,
    },
    SetMetricSystemSymbol {
        metric: MetricKind,
        symbol: SystemSymbolName,
    },
    SetMetricPngMetadata {
        metric: MetricKind,
        png: Option<PngIconMetadata>,
    },
    SetMetricColorMode {
        metric: MetricKind,
        mode: MetricColorMode,
    },
    SetMetricSharedColor {
        metric: MetricKind,
        color: SrgbColor,
    },
    SetMetricVariantsEnabled {
        metric: MetricKind,
        enabled: bool,
    },
    SetMetricAppearanceColor {
        metric: MetricKind,
        appearance: IndicatorAppearance,
        color: SrgbColor,
    },
    SetLabelsVisible(bool),
    SetCpuLabel(IndicatorLabel),
    SetRamLabel(IndicatorLabel),
    SetLabelSpacing(LabelSpacing),
    SetLabelColorMode(LabelColorMode),
    SetLabelSharedColor(SrgbColor),
    SetLabelVariantsEnabled(bool),
    SetLabelAppearanceColor {
        appearance: IndicatorAppearance,
        color: SrgbColor,
    },
    SetFontFamily(FontFamilyPreference),
    SetFontWeight(FontWeight),
    SetRefreshInterval(MetricsRefreshInterval),
    SetFontSize(FontSize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Preferences,
    History,
    FreeSpace,
    SystemUsage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    RequestIndicatorRedraw,
    SetMetricsSamplingInterval(MetricsRefreshInterval),
    ShowWindow(WindowKind),
    QueuePreferencesSave(Preferences),
    FlushPreferences(Preferences),
    ReleasePreferencesWindow,
    SetDiskSamplingEnabled(bool),
    DiskPressureAlert(DiskObservation),
    RequestNotificationAuthorization,
    CheckMoleCompatibility,
    LaunchMoleInTerminal,
    RecordHistory(HistoryEventKind),
    ClearHistory,
    ImportMetricPng {
        metric: MetricKind,
        source: PathBuf,
    },
    CancelMetricPngImport(MetricKind),
    RemoveMetricPngAsset(MetricKind),
    PersistMetricPngChange {
        metric: MetricKind,
        mutation: MetricPngAssetMutation,
        previous: MetricIdentifierPreferences,
        preferences: Preferences,
    },
    PersistIdentifierReset {
        previous: IdentifierPreferences,
        preferences: Preferences,
    },
    PersistGlobalIndicatorReset {
        previous: IndicatorPreferences,
        replaced_undo: Option<IndicatorPreferences>,
        preferences: Preferences,
    },
    PersistGlobalIndicatorUndo {
        current: IndicatorPreferences,
        undo: IndicatorPreferences,
        preferences: Preferences,
    },
    DiscardGlobalIndicatorUndo {
        discarded: IndicatorPreferences,
        preferences: Preferences,
    },
    Quit,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Preferences {
    pub mole_integration_enabled: bool,
    pub warning_threshold: WarningThreshold,
    pub indicator: IndicatorPreferences,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WarningThreshold(u8);

impl WarningThreshold {
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Default for WarningThreshold {
    fn default() -> Self {
        Self(90)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidWarningThreshold(pub u8);

impl TryFrom<u8> for WarningThreshold {
    type Error = InvalidWarningThreshold;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (70..=95).contains(&value) && value.is_multiple_of(5) {
            Ok(Self(value))
        } else {
            Err(InvalidWarningThreshold(value))
        }
    }
}

pub struct StatletCore {
    state: AppState,
    system_snapshot: SystemSnapshot,
    disk_episode: DiskEpisode,
    last_mole_block: Option<HistoryEventKind>,
    monitoring_failure_active: bool,
    indicator_reset_undo: Option<IndicatorPreferences>,
}

impl StatletCore {
    pub fn new() -> Self {
        Self::with_preferences(Preferences::default()).0
    }

    pub fn with_preferences(preferences: Preferences) -> (Self, Vec<AppEffect>) {
        let disk_sampling_enabled = preferences.mole_integration_enabled;
        let system_snapshot = SystemSnapshot {
            cpu_percent: 0.0,
            ram_percent: 0.0,
            memory_pressure: MemoryPressure::Normal,
        };
        let core = Self {
            state: AppState {
                status: present(system_snapshot, None),
                preferences,
                latest_disk_observation: None,
                mole_status: MoleStatus::Unknown,
                can_undo_indicator_reset: false,
                preferences_save_status: PreferencesSaveStatus::Saved,
                indicator_icon_errors: [None, None],
                indicator_icon_error_owners: [None, None],
                indicator_icon_pending: [false; 2],
                system_usage_section: SystemUsageSection::Ram,
            },
            system_snapshot,
            disk_episode: DiskEpisode::default(),
            last_mole_block: None,
            monitoring_failure_active: false,
            indicator_reset_undo: None,
        };
        let mut effects = vec![AppEffect::SetDiskSamplingEnabled(disk_sampling_enabled)];
        if disk_sampling_enabled {
            effects.push(AppEffect::CheckMoleCompatibility);
        }
        (core, effects)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn handle(&mut self, event: AppEvent) -> Vec<AppEffect> {
        match event {
            AppEvent::ApplicationLaunched => {
                vec![AppEffect::ShowWindow(WindowKind::Preferences)]
            }
            AppEvent::ApplicationReopened {
                has_visible_windows: false,
            } => vec![AppEffect::ShowWindow(WindowKind::Preferences)],
            AppEvent::ApplicationReopened {
                has_visible_windows: true,
            } => Vec::new(),
            AppEvent::MetricsSample(snapshot) => {
                self.system_snapshot = snapshot;
                self.refresh_status();
                Vec::new()
            }
            AppEvent::OpenPreferences => {
                vec![AppEffect::ShowWindow(WindowKind::Preferences)]
            }
            AppEvent::OpenHistory => vec![AppEffect::ShowWindow(WindowKind::History)],
            AppEvent::OpenSystemUsage => {
                vec![AppEffect::ShowWindow(WindowKind::SystemUsage)]
            }
            AppEvent::SelectSystemUsageSection(section) => {
                self.state.system_usage_section = section;
                Vec::new()
            }
            AppEvent::ReviewSpace | AppEvent::NotificationActivated => {
                let mut effects = Vec::with_capacity(3);
                if self.state.preferences.mole_integration_enabled {
                    self.state.mole_status = MoleStatus::Unknown;
                    if self.refresh_status() {
                        effects.push(AppEffect::RequestIndicatorRedraw);
                    }
                    effects.push(AppEffect::CheckMoleCompatibility);
                }
                effects.push(AppEffect::ShowWindow(WindowKind::FreeSpace));
                effects
            }
            AppEvent::Quit => vec![
                AppEffect::FlushPreferences(self.state.preferences.clone()),
                AppEffect::Quit,
            ],
            AppEvent::SetMoleIntegrationEnabled(enabled) => {
                if self.state.preferences.mole_integration_enabled == enabled {
                    return Vec::new();
                }
                self.state.preferences.mole_integration_enabled = enabled;
                let disk_badge_changed = if !enabled {
                    self.disk_episode = DiskEpisode::default();
                    self.state.latest_disk_observation = None;
                    self.state.mole_status = MoleStatus::Unknown;
                    self.last_mole_block = None;
                    self.monitoring_failure_active = false;
                    self.refresh_status()
                } else {
                    false
                };
                let mut effects = vec![
                    AppEffect::QueuePreferencesSave(self.state.preferences.clone()),
                    AppEffect::SetDiskSamplingEnabled(enabled),
                ];
                if disk_badge_changed {
                    effects.insert(0, AppEffect::RequestIndicatorRedraw);
                }
                if enabled {
                    effects.push(AppEffect::RequestNotificationAuthorization);
                    effects.push(AppEffect::CheckMoleCompatibility);
                }
                effects
            }
            AppEvent::SetWarningThreshold(threshold) => {
                if self.state.preferences.warning_threshold == threshold {
                    return Vec::new();
                }
                self.state.preferences.warning_threshold = threshold;
                vec![AppEffect::QueuePreferencesSave(
                    self.state.preferences.clone(),
                )]
            }
            AppEvent::DiskObserved(observation) => {
                if !self.state.preferences.mole_integration_enabled {
                    return Vec::new();
                }
                self.state.latest_disk_observation = Some(observation);
                self.monitoring_failure_active = false;
                let transition = self
                    .disk_episode
                    .observe(observation, self.state.preferences.warning_threshold);
                let status_changed = self.refresh_status();
                let mut effects = match transition {
                    DiskEpisodeTransition::Started => vec![
                        AppEffect::RecordHistory(HistoryEventKind::DiskPressureStarted),
                        AppEffect::DiskPressureAlert(observation),
                    ],
                    DiskEpisodeTransition::Recovered => vec![AppEffect::RecordHistory(
                        HistoryEventKind::DiskPressureRecovered,
                    )],
                    DiskEpisodeTransition::None => Vec::new(),
                };
                if status_changed {
                    effects.insert(0, AppEffect::RequestIndicatorRedraw);
                }
                effects
            }
            AppEvent::DiskMonitoringFailed => {
                if !self.state.preferences.mole_integration_enabled
                    || self.monitoring_failure_active
                {
                    return Vec::new();
                }
                self.monitoring_failure_active = true;
                vec![AppEffect::RecordHistory(HistoryEventKind::MonitoringFailed)]
            }
            AppEvent::MoleStatusObserved(status) => {
                if !self.state.preferences.mole_integration_enabled {
                    return Vec::new();
                }
                self.state.mole_status = status;
                let disk_badge_changed = self.refresh_status();
                let block = match status {
                    MoleStatus::Missing => Some(HistoryEventKind::MoleMissing),
                    MoleStatus::Unavailable => Some(HistoryEventKind::MoleUnavailable),
                    MoleStatus::Incompatible(_) => Some(HistoryEventKind::MoleIncompatible),
                    MoleStatus::Compatible(_) => {
                        self.last_mole_block = None;
                        None
                    }
                    MoleStatus::Unknown => None,
                };
                let mut effects = match block {
                    Some(block) if Some(block) != self.last_mole_block => {
                        self.last_mole_block = Some(block);
                        vec![AppEffect::RecordHistory(block)]
                    }
                    _ => Vec::new(),
                };
                if disk_badge_changed {
                    effects.insert(0, AppEffect::RequestIndicatorRedraw);
                }
                effects
            }
            AppEvent::OpenMoleInTerminal => {
                if self.state.preferences.mole_integration_enabled
                    && self.state.mole_status.is_compatible()
                {
                    vec![AppEffect::LaunchMoleInTerminal]
                } else {
                    Vec::new()
                }
            }
            AppEvent::ClearHistoryConfirmed => vec![AppEffect::ClearHistory],
            AppEvent::ChooseMetricPng { metric, source } => {
                self.clear_indicator_icon_error(metric);
                let mut effects = self.cancel_pending_png_imports([metric]);
                self.state.indicator_icon_pending[metric_index(metric)] = true;
                effects.push(AppEffect::ImportMetricPng { metric, source });
                effects
            }
            AppEvent::CancelMetricPngImport(metric) => self.cancel_pending_png_imports([metric]),
            AppEvent::MetricPngImportFinished { metric, result } => match result {
                MetricPngImportResult::Imported(metadata) => {
                    self.state.indicator_icon_pending[metric_index(metric)] = false;
                    let previous_interval = self.state.preferences.indicator.refresh_interval;
                    let previous =
                        metric_identifier(&mut self.state.preferences.indicator, metric).clone();
                    let identifier =
                        metric_identifier(&mut self.state.preferences.indicator, metric);
                    identifier.mode = MetricIdentifierMode::Png;
                    identifier.png = Some(metadata);
                    self.clear_indicator_icon_error(metric);
                    let mut effects = Vec::with_capacity(3);
                    let current_interval = self.state.preferences.indicator.refresh_interval;
                    if current_interval != previous_interval {
                        effects.push(AppEffect::SetMetricsSamplingInterval(current_interval));
                    }
                    effects.push(AppEffect::RequestIndicatorRedraw);
                    effects.push(AppEffect::PersistMetricPngChange {
                        metric,
                        mutation: MetricPngAssetMutation::Replace,
                        previous,
                        preferences: self.state.preferences.clone(),
                    });
                    effects
                }
                MetricPngImportResult::Failed(message) => {
                    self.state.indicator_icon_pending[metric_index(metric)] = false;
                    self.set_indicator_icon_error(
                        metric,
                        message,
                        IndicatorIconErrorOwner::Independent,
                    );
                    Vec::new()
                }
            },
            AppEvent::RemoveMetricPng(metric) => {
                if metric_identifier(&mut self.state.preferences.indicator, metric)
                    .png
                    .is_none()
                {
                    Vec::new()
                } else {
                    self.clear_indicator_icon_error(metric);
                    vec![AppEffect::RemoveMetricPngAsset(metric)]
                }
            }
            AppEvent::MetricPngRemovalFinished { metric, result } => match result {
                MetricPngRemovalResult::Removed => {
                    let previous_interval = self.state.preferences.indicator.refresh_interval;
                    let previous =
                        metric_identifier(&mut self.state.preferences.indicator, metric).clone();
                    let identifier =
                        metric_identifier(&mut self.state.preferences.indicator, metric);
                    let changed =
                        identifier.mode != MetricIdentifierMode::Text || identifier.png.is_some();
                    identifier.mode = MetricIdentifierMode::Text;
                    identifier.png = None;
                    self.clear_indicator_icon_error(metric);
                    if changed {
                        let mut effects = Vec::with_capacity(3);
                        let current_interval = self.state.preferences.indicator.refresh_interval;
                        if current_interval != previous_interval {
                            effects.push(AppEffect::SetMetricsSamplingInterval(current_interval));
                        }
                        effects.push(AppEffect::RequestIndicatorRedraw);
                        effects.push(AppEffect::PersistMetricPngChange {
                            metric,
                            mutation: MetricPngAssetMutation::Remove,
                            previous,
                            preferences: self.state.preferences.clone(),
                        });
                        effects
                    } else {
                        Vec::new()
                    }
                }
                MetricPngRemovalResult::Failed(message) => {
                    self.set_indicator_icon_error(
                        metric,
                        message,
                        IndicatorIconErrorOwner::Independent,
                    );
                    Vec::new()
                }
            },
            AppEvent::MetricPngPersistenceFailed {
                metric,
                previous,
                message,
            } => {
                self.state.indicator_icon_pending[metric_index(metric)] = false;
                *metric_identifier(&mut self.state.preferences.indicator, metric) = previous;
                self.state.preferences_save_status = PreferencesSaveStatus::Failed;
                self.set_indicator_icon_error(
                    metric,
                    message,
                    IndicatorIconErrorOwner::Independent,
                );
                vec![AppEffect::RequestIndicatorRedraw]
            }
            AppEvent::MetricPngTransactionCleanupFailed { metric, message } => {
                self.set_indicator_icon_error(
                    metric,
                    message,
                    IndicatorIconErrorOwner::Independent,
                );
                Vec::new()
            }
            AppEvent::MetricPngDurabilityWarning { metric, message } => {
                self.set_indicator_icon_error(
                    metric,
                    message,
                    IndicatorIconErrorOwner::PreferencesDurability,
                );
                Vec::new()
            }
            AppEvent::IdentifierResetPersistenceFailed { previous, message } => {
                let affected = [
                    (MetricKind::Cpu, previous.cpu.png.is_some()),
                    (MetricKind::Ram, previous.ram.png.is_some()),
                ];
                self.state.preferences.indicator.identifiers = previous;
                self.state.preferences_save_status = PreferencesSaveStatus::Failed;
                for (metric, has_png) in affected {
                    if has_png {
                        self.set_indicator_icon_error(
                            metric,
                            message.clone(),
                            IndicatorIconErrorOwner::Independent,
                        );
                    }
                }
                vec![AppEffect::RequestIndicatorRedraw]
            }
            AppEvent::GlobalIndicatorResetPersistenceFailed(failure) => {
                let GlobalIndicatorResetFailure {
                    previous,
                    replaced_undo,
                    message,
                } = *failure;
                let failed_interval = self.state.preferences.indicator.refresh_interval;
                self.state.preferences.indicator = previous.clone();
                self.indicator_reset_undo = replaced_undo;
                self.state.can_undo_indicator_reset = self.indicator_reset_undo.is_some();
                self.state.preferences_save_status = PreferencesSaveStatus::Failed;
                self.set_global_indicator_asset_error(&previous, message);
                self.failed_indicator_persistence_effects(failed_interval)
            }
            AppEvent::GlobalIndicatorUndoPersistenceFailed(failure) => {
                let GlobalIndicatorUndoFailure {
                    current,
                    undo,
                    message,
                    stage,
                } = *failure;
                let failed_interval = self.state.preferences.indicator.refresh_interval;
                self.state.preferences.indicator = current.clone();
                self.set_global_indicator_asset_error_for(&[&current, &undo], message);
                self.indicator_reset_undo = Some(undo);
                self.state.can_undo_indicator_reset = true;
                if stage == GlobalIndicatorUndoFailureStage::Persistence {
                    self.state.preferences_save_status = PreferencesSaveStatus::Failed;
                }
                self.failed_indicator_persistence_effects(failed_interval)
            }
            AppEvent::UpdateIndicator(change) => {
                let previous_interval = self.state.preferences.indicator.refresh_interval;
                let explicit_identifier_mode_metric = match &change {
                    IndicatorPreferenceChange::SetMetricIdentifierMode { metric, .. } => {
                        Some(*metric)
                    }
                    _ => None,
                };
                let identifier_metric = match &change {
                    IndicatorPreferenceChange::SetMetricIdentifierMode { metric, .. }
                    | IndicatorPreferenceChange::SetMetricSystemSymbol { metric, .. }
                    | IndicatorPreferenceChange::SetMetricPngMetadata { metric, .. } => {
                        Some(*metric)
                    }
                    _ => None,
                };
                if let Some(metric) = explicit_identifier_mode_metric {
                    self.clear_stale_indicator_icon_error(metric);
                }
                if !change.apply(&mut self.state.preferences.indicator) {
                    return explicit_identifier_mode_metric
                        .map(|metric| self.cancel_pending_png_imports([metric]))
                        .unwrap_or_default();
                }
                if let Some(metric) = identifier_metric {
                    self.clear_indicator_icon_error(metric);
                }
                let mut effects = self.indicator_effects(previous_interval);
                if let Some(metric) = identifier_metric
                    .filter(|metric| self.state.indicator_icon_pending[metric_index(*metric)])
                {
                    self.state.indicator_icon_pending[metric_index(metric)] = false;
                    effects.insert(0, AppEffect::CancelMetricPngImport(metric));
                }
                effects
            }
            AppEvent::ResetIndicatorGroup(group) => {
                let previous = self.state.preferences.indicator.clone();
                self.state.preferences.indicator.reset(group);
                let mut effects = if group == IndicatorPreferenceGroup::Identifiers {
                    for metric in [MetricKind::Cpu, MetricKind::Ram] {
                        self.clear_stale_indicator_icon_error(metric);
                    }
                    self.cancel_pending_png_imports([MetricKind::Cpu, MetricKind::Ram])
                } else {
                    Vec::new()
                };
                if self.state.preferences.indicator == previous {
                    return effects;
                }
                if group == IndicatorPreferenceGroup::Identifiers {
                    effects.push(AppEffect::RequestIndicatorRedraw);
                    effects.push(AppEffect::PersistIdentifierReset {
                        previous: previous.identifiers,
                        preferences: self.state.preferences.clone(),
                    });
                } else {
                    effects.extend(self.indicator_effects(previous.refresh_interval));
                }
                effects
            }
            AppEvent::ResetIndicatorConfirmed => {
                let previous = self.state.preferences.indicator.clone();
                if previous == IndicatorPreferences::default()
                    && self.indicator_reset_undo.is_none()
                {
                    for metric in [MetricKind::Cpu, MetricKind::Ram] {
                        self.clear_stale_indicator_icon_error(metric);
                    }
                    return self.cancel_pending_png_imports([MetricKind::Cpu, MetricKind::Ram]);
                }
                let replaced_undo = self.indicator_reset_undo.replace(previous.clone());
                self.state.can_undo_indicator_reset = true;
                self.state.preferences.indicator = IndicatorPreferences::default();
                for metric in [MetricKind::Cpu, MetricKind::Ram] {
                    self.clear_stale_indicator_icon_error(metric);
                }
                let mut effects =
                    self.cancel_pending_png_imports([MetricKind::Cpu, MetricKind::Ram]);
                if self.state.preferences.indicator == previous && replaced_undo.is_none() {
                    return effects;
                }
                if self.state.preferences.indicator != previous {
                    let current_interval = self.state.preferences.indicator.refresh_interval;
                    if current_interval != previous.refresh_interval {
                        effects.push(AppEffect::SetMetricsSamplingInterval(current_interval));
                    }
                    effects.push(AppEffect::RequestIndicatorRedraw);
                }
                effects.push(AppEffect::PersistGlobalIndicatorReset {
                    previous,
                    replaced_undo,
                    preferences: self.state.preferences.clone(),
                });
                effects
            }
            AppEvent::UndoIndicatorReset => {
                let Some(previous) = self.indicator_reset_undo.take() else {
                    return Vec::new();
                };
                self.state.can_undo_indicator_reset = false;
                let current = self.state.preferences.indicator.clone();
                let current_interval = self.state.preferences.indicator.refresh_interval;
                let mut effects = Vec::new();
                if current != previous {
                    self.state.preferences.indicator = previous.clone();
                    let restored_interval = self.state.preferences.indicator.refresh_interval;
                    if restored_interval != current_interval {
                        effects.push(AppEffect::SetMetricsSamplingInterval(restored_interval));
                    }
                    effects.push(AppEffect::RequestIndicatorRedraw);
                }
                effects.push(AppEffect::PersistGlobalIndicatorUndo {
                    current,
                    undo: previous,
                    preferences: self.state.preferences.clone(),
                });
                effects
            }
            AppEvent::PreferencesWindowClosed => {
                let discarded = self.indicator_reset_undo.take();
                self.state.can_undo_indicator_reset = false;
                let persistence = discarded.map_or_else(
                    || AppEffect::FlushPreferences(self.state.preferences.clone()),
                    |discarded| AppEffect::DiscardGlobalIndicatorUndo {
                        discarded,
                        preferences: self.state.preferences.clone(),
                    },
                );
                vec![persistence, AppEffect::ReleasePreferencesWindow]
            }
            AppEvent::RetrySavePreferences => {
                vec![AppEffect::FlushPreferences(self.state.preferences.clone())]
            }
            AppEvent::PreferencesSaveFinished(result) => {
                self.state.preferences_save_status = match result {
                    PreferencesSaveResult::Saved => {
                        self.clear_resolved_durability_warnings();
                        PreferencesSaveStatus::Saved
                    }
                    PreferencesSaveResult::Failed => PreferencesSaveStatus::Failed,
                };
                Vec::new()
            }
        }
    }

    fn clear_indicator_icon_error(&mut self, metric: MetricKind) {
        let index = metric_index(metric);
        self.state.indicator_icon_errors[index] = None;
        self.state.indicator_icon_error_owners[index] = None;
    }

    fn clear_stale_indicator_icon_error(&mut self, metric: MetricKind) {
        let index = metric_index(metric);
        if self.state.indicator_icon_error_owners[index]
            == Some(IndicatorIconErrorOwner::Independent)
        {
            self.clear_indicator_icon_error(metric);
        }
    }

    fn set_indicator_icon_error(
        &mut self,
        metric: MetricKind,
        message: String,
        owner: IndicatorIconErrorOwner,
    ) {
        let index = metric_index(metric);
        self.state.indicator_icon_errors[index] = Some(message);
        self.state.indicator_icon_error_owners[index] = Some(owner);
    }

    fn set_global_indicator_asset_error(
        &mut self,
        indicator: &IndicatorPreferences,
        message: String,
    ) {
        self.set_global_indicator_asset_error_for(&[indicator], message);
    }

    fn set_global_indicator_asset_error_for(
        &mut self,
        indicators: &[&IndicatorPreferences],
        message: String,
    ) {
        for metric in [MetricKind::Cpu, MetricKind::Ram] {
            let has_png = indicators.iter().any(|indicator| match metric {
                MetricKind::Cpu => indicator.identifiers.cpu.png.is_some(),
                MetricKind::Ram => indicator.identifiers.ram.png.is_some(),
            });
            if has_png {
                self.set_indicator_icon_error(
                    metric,
                    message.clone(),
                    IndicatorIconErrorOwner::PreferencesDurability,
                );
            }
        }
    }

    fn failed_indicator_persistence_effects(
        &self,
        failed_interval: MetricsRefreshInterval,
    ) -> Vec<AppEffect> {
        let restored_interval = self.state.preferences.indicator.refresh_interval;
        let mut effects = Vec::with_capacity(2);
        if restored_interval != failed_interval {
            effects.push(AppEffect::SetMetricsSamplingInterval(restored_interval));
        }
        effects.push(AppEffect::RequestIndicatorRedraw);
        effects
    }

    fn clear_resolved_durability_warnings(&mut self) {
        for index in 0..self.state.indicator_icon_errors.len() {
            if self.state.indicator_icon_error_owners[index]
                == Some(IndicatorIconErrorOwner::PreferencesDurability)
            {
                self.state.indicator_icon_errors[index] = None;
                self.state.indicator_icon_error_owners[index] = None;
            }
        }
    }

    fn cancel_pending_png_imports(
        &mut self,
        metrics: impl IntoIterator<Item = MetricKind>,
    ) -> Vec<AppEffect> {
        metrics
            .into_iter()
            .filter_map(|metric| {
                let pending = &mut self.state.indicator_icon_pending[metric_index(metric)];
                if *pending {
                    *pending = false;
                    Some(AppEffect::CancelMetricPngImport(metric))
                } else {
                    None
                }
            })
            .collect()
    }

    fn indicator_effects(&self, previous_interval: MetricsRefreshInterval) -> Vec<AppEffect> {
        let mut effects = Vec::with_capacity(3);
        let current_interval = self.state.preferences.indicator.refresh_interval;
        if current_interval != previous_interval {
            effects.push(AppEffect::SetMetricsSamplingInterval(current_interval));
        }
        effects.push(AppEffect::RequestIndicatorRedraw);
        effects.push(AppEffect::QueuePreferencesSave(
            self.state.preferences.clone(),
        ));
        effects
    }

    fn refresh_status(&mut self) -> bool {
        let previous_status = self.state.status.clone();
        let disk_badge = if self.state.preferences.mole_integration_enabled
            && self.state.mole_status.is_error()
        {
            Some(DiskBadge::Error)
        } else {
            self.disk_episode.is_active().then_some(DiskBadge::Warning)
        };
        self.state.status = present(self.system_snapshot, disk_badge);
        self.state.status != previous_status
    }
}

impl IndicatorPreferenceChange {
    fn apply(self, indicator: &mut IndicatorPreferences) -> bool {
        match self {
            Self::SetMetricIdentifierMode { metric, mode } => {
                replace_if_changed(&mut metric_identifier(indicator, metric).mode, mode)
            }
            Self::SetMetricSystemSymbol { metric, symbol } => replace_if_changed(
                &mut metric_identifier(indicator, metric).system_symbol,
                symbol,
            ),
            Self::SetMetricPngMetadata { metric, png } => {
                replace_if_changed(&mut metric_identifier(indicator, metric).png, png)
            }
            Self::SetMetricColorMode { metric, mode } => {
                replace_if_changed(&mut metric_colors(indicator, metric).mode, mode)
            }
            Self::SetMetricSharedColor { metric, color } => {
                replace_if_changed(&mut metric_colors(indicator, metric).fixed.shared, color)
            }
            Self::SetMetricVariantsEnabled { metric, enabled } => {
                let fixed = &mut metric_colors(indicator, metric).fixed;
                let before = *fixed;
                fixed.set_variants_enabled(enabled);
                *fixed != before
            }
            Self::SetMetricAppearanceColor {
                metric,
                appearance,
                color,
            } => {
                let fixed = &mut metric_colors(indicator, metric).fixed;
                set_appearance_color(fixed.shared, &mut fixed.variants, appearance, color)
            }
            Self::SetLabelsVisible(visible) => {
                replace_if_changed(&mut indicator.labels.visible, visible)
            }
            Self::SetCpuLabel(label) => replace_if_changed(&mut indicator.labels.cpu, label),
            Self::SetRamLabel(label) => replace_if_changed(&mut indicator.labels.ram, label),
            Self::SetLabelSpacing(spacing) => {
                replace_if_changed(&mut indicator.labels.spacing, spacing)
            }
            Self::SetLabelColorMode(mode) => {
                replace_if_changed(&mut indicator.labels.color_mode, mode)
            }
            Self::SetLabelSharedColor(color) => {
                replace_if_changed(&mut indicator.labels.fixed.shared, color)
            }
            Self::SetLabelVariantsEnabled(enabled) => {
                let fixed = &mut indicator.labels.fixed;
                let before = *fixed;
                fixed.set_variants_enabled(enabled);
                *fixed != before
            }
            Self::SetLabelAppearanceColor { appearance, color } => {
                let fixed = &mut indicator.labels.fixed;
                set_appearance_color(fixed.shared, &mut fixed.variants, appearance, color)
            }
            Self::SetFontFamily(family) => {
                replace_if_changed(&mut indicator.typography.family, family)
            }
            Self::SetFontWeight(weight) => {
                replace_if_changed(&mut indicator.typography.weight, weight)
            }
            Self::SetRefreshInterval(interval) => {
                replace_if_changed(&mut indicator.refresh_interval, interval)
            }
            Self::SetFontSize(size) => replace_if_changed(&mut indicator.typography.size, size),
        }
    }
}

fn metric_identifier(
    indicator: &mut IndicatorPreferences,
    metric: MetricKind,
) -> &mut MetricIdentifierPreferences {
    match metric {
        MetricKind::Cpu => &mut indicator.identifiers.cpu,
        MetricKind::Ram => &mut indicator.identifiers.ram,
    }
}

const fn metric_index(metric: MetricKind) -> usize {
    match metric {
        MetricKind::Cpu => 0,
        MetricKind::Ram => 1,
    }
}

fn metric_colors(
    indicator: &mut IndicatorPreferences,
    metric: MetricKind,
) -> &mut MetricColorPreferences {
    match metric {
        MetricKind::Cpu => &mut indicator.cpu_color,
        MetricKind::Ram => &mut indicator.ram_color,
    }
}

fn replace_if_changed<T: PartialEq>(target: &mut T, value: T) -> bool {
    if *target == value {
        return false;
    }
    *target = value;
    true
}

fn set_appearance_color(
    shared: SrgbColor,
    variants: &mut Option<AppearanceColors>,
    appearance: IndicatorAppearance,
    color: SrgbColor,
) -> bool {
    if variants.is_none() && color == shared {
        return false;
    }
    let variants = variants.get_or_insert(AppearanceColors {
        light: shared,
        dark: shared,
    });
    let target = match appearance {
        IndicatorAppearance::Light => &mut variants.light,
        IndicatorAppearance::Dark => &mut variants.dark,
    };
    if *target == color {
        return false;
    }
    *target = color;
    true
}

impl Default for StatletCore {
    fn default() -> Self {
        Self::new()
    }
}

const REQUIRED_PRESSURE_DURATION: Duration = Duration::from_secs(5 * 60);
const MAX_OBSERVED_GAP: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DiskEpisode {
    #[default]
    Ready,
    Debouncing {
        started_at: Duration,
        last_observed_at: Duration,
    },
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiskEpisodeTransition {
    None,
    Started,
    Recovered,
}

impl DiskEpisode {
    fn observe(
        &mut self,
        observation: DiskObservation,
        threshold: WarningThreshold,
    ) -> DiskEpisodeTransition {
        if !observation.is_at_or_above(threshold.get()) {
            let recovered = matches!(self, Self::Active);
            *self = Self::Ready;
            return if recovered {
                DiskEpisodeTransition::Recovered
            } else {
                DiskEpisodeTransition::None
            };
        }

        let observed_at = observation.observed_at();
        match *self {
            Self::Ready => {
                *self = Self::Debouncing {
                    started_at: observed_at,
                    last_observed_at: observed_at,
                };
                DiskEpisodeTransition::None
            }
            Self::Debouncing {
                started_at,
                last_observed_at,
            } => {
                let continuous = observed_at
                    .checked_sub(last_observed_at)
                    .is_some_and(|gap| gap <= MAX_OBSERVED_GAP);
                if !continuous {
                    *self = Self::Debouncing {
                        started_at: observed_at,
                        last_observed_at: observed_at,
                    };
                    return DiskEpisodeTransition::None;
                }

                if observed_at.saturating_sub(started_at) >= REQUIRED_PRESSURE_DURATION {
                    *self = Self::Active;
                    DiskEpisodeTransition::Started
                } else {
                    *self = Self::Debouncing {
                        started_at,
                        last_observed_at: observed_at,
                    };
                    DiskEpisodeTransition::None
                }
            }
            Self::Active => DiskEpisodeTransition::None,
        }
    }

    fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

fn present(snapshot: SystemSnapshot, disk_badge: Option<DiskBadge>) -> StatusContent {
    let cpu = rounded_percent(snapshot.cpu_percent);
    let ram = rounded_percent(snapshot.ram_percent);
    let (memory_severity, pressure_description) =
        memory_pressure_presentation(snapshot.memory_pressure);
    let disk_description = match disk_badge {
        Some(DiskBadge::Warning) => ", disco acima do limite",
        Some(DiskBadge::Error) => ", Mole indisponível",
        None => "",
    };
    StatusContent {
        cpu: MetricContent {
            label: "C",
            percent: cpu,
            severity: cpu_severity(snapshot.cpu_percent),
        },
        ram: MetricContent {
            label: "R",
            percent: ram,
            severity: memory_severity,
        },
        disk_badge,
        accessibility_label: format!(
            "CPU {cpu}%, RAM {ram}%, pressão de memória {pressure_description}{disk_description}"
        ),
    }
}

fn rounded_percent(value: f64) -> u8 {
    value.clamp(0.0, 100.0).round() as u8
}

fn cpu_severity(percent: f64) -> MetricSeverity {
    match percent {
        value if value < 40.0 => MetricSeverity::Good,
        value if value < 70.0 => MetricSeverity::Warning,
        _ => MetricSeverity::Critical,
    }
}

fn memory_pressure_presentation(pressure: MemoryPressure) -> (MetricSeverity, &'static str) {
    match pressure {
        MemoryPressure::Normal => (MetricSeverity::Good, "normal"),
        MemoryPressure::Warning => (MetricSeverity::Warning, "em atenção"),
        MemoryPressure::Critical => (MetricSeverity::Critical, "crítica"),
    }
}
