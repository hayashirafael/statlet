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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewBackground {
    Light,
    Dark,
}

impl PreviewBackground {
    pub const fn components(self) -> [f64; 3] {
        match self {
            Self::Light => [1.0, 1.0, 1.0],
            Self::Dark => [30.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0],
        }
    }
}

pub fn has_low_text_contrast(colors: &[[f64; 3]], background: PreviewBackground) -> bool {
    const SMALL_TEXT_CONTRAST_THRESHOLD: f64 = 4.5;

    colors.iter().any(|color| {
        let foreground_luminance = relative_luminance(*color);
        let background_luminance = relative_luminance(background.components());
        let ratio = (foreground_luminance.max(background_luminance) + 0.05)
            / (foreground_luminance.min(background_luminance) + 0.05);
        ratio < SMALL_TEXT_CONTRAST_THRESHOLD
    })
}

fn relative_luminance(color: [f64; 3]) -> f64 {
    let [red, green, blue] = color.map(|component| {
        if component <= 0.04045 {
            component / 12.92
        } else {
            ((component + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * red + 0.7152 * green + 0.0722 * blue
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

pub fn preview_accessibility_summary(
    scene: &IndicatorScene,
    resolved_colors: &[[f64; 3]],
    appearance: IndicatorAppearance,
) -> String {
    let mut color_index = 0;
    let (cpu, cpu_has_label) =
        preview_metric_summary("CPU", &scene.top, resolved_colors, &mut color_index);
    let (ram, ram_has_label) =
        preview_metric_summary("RAM", &scene.bottom, resolved_colors, &mut color_index);
    let labels = if cpu_has_label && ram_has_label {
        "rótulos exibidos"
    } else {
        "rótulos ocultos"
    };
    let badge = match &scene.disk_badge {
        Some(run) => {
            let state = match run.text.trim() {
                "!" => "atenção",
                "×" => "erro",
                _ => "estado",
            };
            let color = resolved_colors
                .get(color_index)
                .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
            format!("badge de {state} presente na cor {color}")
        }
        None => "badge ausente".to_owned(),
    };
    let appearance = match appearance {
        IndicatorAppearance::Light => "clara",
        IndicatorAppearance::Dark => "escura",
    };

    format!("Prévia {appearance}: {cpu}; {ram}; {labels}; {badge}.")
}

fn preview_metric_summary(
    name: &str,
    runs: &[IndicatorRun],
    resolved_colors: &[[f64; 3]],
    color_index: &mut usize,
) -> (String, bool) {
    match runs {
        [_, value] => {
            let label_color = resolved_colors
                .get(*color_index)
                .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
            *color_index += 1;
            let value_color = resolved_colors
                .get(*color_index)
                .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
            *color_index += 1;
            (
                format!(
                    "{name} {}, rótulo {label_color} e valor {value_color}",
                    value.text.trim()
                ),
                true,
            )
        }
        [value] => {
            let value_color = resolved_colors
                .get(*color_index)
                .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
            *color_index += 1;
            (
                format!("{name} {}, valor {value_color}", value.text.trim()),
                false,
            )
        }
        _ => (format!("{name} indisponível"), false),
    }
}

fn color_hex(components: [f64; 3]) -> String {
    let [red, green, blue] =
        components.map(|component| (component.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{red:02X}{green:02X}{blue:02X}")
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

    pub fn value_origin(&self, measurer: &impl TextMeasurer, line_width: f64, value: &str) -> f64 {
        line_width - measurer.width(value)
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

#[cfg(test)]
mod tests {
    use super::{has_low_text_contrast, PreviewBackground};

    #[test]
    fn shared_contrast_function_applies_wcag_small_text_threshold_to_both_backgrounds() {
        let gray = 119.0 / 255.0;
        assert!(has_low_text_contrast(
            &[[gray, gray, gray]],
            PreviewBackground::Light
        ));
        assert!(!has_low_text_contrast(
            &[[1.0, 1.0, 1.0]],
            PreviewBackground::Dark
        ));
    }
}
