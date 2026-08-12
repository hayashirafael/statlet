use crate::core::{DiskBadge, MetricContent, MetricSeverity, StatusContent};
use crate::indicator_preferences::{
    IndicatorAppearance, IndicatorPreferences, LabelColorMode, MetricColorMode,
    MetricColorPreferences, SrgbColor,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticColor {
    Neutral,
    Good,
    Warning,
    Critical,
    DiskWarning,
    DiskError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentColor {
    Semantic(SemanticColor),
    Srgb(SrgbColor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorRun {
    pub text: String,
    pub color: SegmentColor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorScene {
    pub top: Vec<IndicatorRun>,
    pub bottom: Vec<IndicatorRun>,
    pub disk_badge: Option<IndicatorRun>,
    pub accessibility_label: String,
}

pub fn compose_indicator(
    status: &StatusContent,
    preferences: &IndicatorPreferences,
    appearance: IndicatorAppearance,
) -> IndicatorScene {
    let cpu_color = metric_color(status.cpu.severity, preferences.cpu_color, appearance);
    let ram_color = metric_color(status.ram.severity, preferences.ram_color, appearance);

    IndicatorScene {
        top: metric_runs(&status.cpu, cpu_color, preferences, appearance),
        bottom: metric_runs(&status.ram, ram_color, preferences, appearance),
        disk_badge: status.disk_badge.map(|badge| match badge {
            DiskBadge::Warning => IndicatorRun {
                text: " !".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::DiskWarning),
            },
            DiskBadge::Error => IndicatorRun {
                text: " ×".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::DiskError),
            },
        }),
        accessibility_label: status.accessibility_label.clone(),
    }
}

fn metric_runs(
    metric: &MetricContent,
    metric_color: SegmentColor,
    preferences: &IndicatorPreferences,
    appearance: IndicatorAppearance,
) -> Vec<IndicatorRun> {
    let mut runs = Vec::with_capacity(2);
    if preferences.labels.visible {
        let color = match preferences.labels.color_mode {
            LabelColorMode::Neutral => SegmentColor::Semantic(SemanticColor::Neutral),
            LabelColorMode::MatchMetric => metric_color,
            LabelColorMode::Fixed => {
                SegmentColor::Srgb(preferences.labels.fixed.color_for(appearance))
            }
        };
        runs.push(IndicatorRun {
            text: format!("{} ", metric.label),
            color,
        });
    }
    runs.push(IndicatorRun {
        text: format!("{}%", metric.percent),
        color: metric_color,
    });
    runs
}

fn metric_color(
    severity: MetricSeverity,
    preferences: MetricColorPreferences,
    appearance: IndicatorAppearance,
) -> SegmentColor {
    match preferences.mode {
        MetricColorMode::Dynamic => SegmentColor::Semantic(match severity {
            MetricSeverity::Good => SemanticColor::Good,
            MetricSeverity::Warning => SemanticColor::Warning,
            MetricSeverity::Critical => SemanticColor::Critical,
        }),
        MetricColorMode::Fixed => SegmentColor::Srgb(preferences.fixed.color_for(appearance)),
    }
}

pub trait TextMeasurer {
    fn width(&self, text: &str) -> f64;
    fn content_height(&self) -> f64;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutDiagnostics {
    pub exceeds_menu_bar_height: bool,
    pub exceeds_curated_width: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StableLayout {
    pub cpu_width: f64,
    pub ram_width: f64,
    pub diagnostics: LayoutDiagnostics,
    warning_badge_width: f64,
    error_badge_width: f64,
}

impl StableLayout {
    pub fn base_width(&self) -> f64 {
        self.cpu_width.max(self.ram_width)
    }

    pub fn width_for_badge(&self, badge: Option<&str>) -> f64 {
        let badge_width = match badge {
            Some(" !") => self.warning_badge_width,
            Some(" ×") => self.error_badge_width,
            _ => 0.0,
        };
        self.base_width() + badge_width
    }
}

pub fn measure_stable_layout(
    measurer: &impl TextMeasurer,
    labels_visible: bool,
    default_width: f64,
) -> StableLayout {
    let cpu_width = widest_metric_width(measurer, "C", labels_visible);
    let ram_width = widest_metric_width(measurer, "R", labels_visible);
    let base_width = cpu_width.max(ram_width);

    StableLayout {
        cpu_width,
        ram_width,
        warning_badge_width: measurer.width(" !"),
        error_badge_width: measurer.width(" ×"),
        diagnostics: LayoutDiagnostics {
            exceeds_menu_bar_height: measurer.content_height() > 22.0,
            exceeds_curated_width: base_width > 2.0 * default_width,
        },
    }
}

fn widest_metric_width(measurer: &impl TextMeasurer, label: &str, labels_visible: bool) -> f64 {
    (0..=100)
        .map(|percent| {
            let text = if labels_visible {
                format!("{label} {percent}%")
            } else {
                format!("{percent}%")
            };
            measurer.width(&text)
        })
        .fold(0.0, f64::max)
}
