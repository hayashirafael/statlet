use crate::core::{DiskBadge, MetricContent, MetricSeverity, StatusContent};
use crate::indicator_preferences::{
    IndicatorAppearance, IndicatorPreferences, LabelColorMode, MetricColorMode,
    MetricColorPreferences, MetricIdentifierMode, MetricKind, PngIconMetadata, SrgbColor,
    SystemSymbolName,
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
    pub trailing_spacing_level: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorScene {
    pub top: Vec<IndicatorRun>,
    pub bottom: Vec<IndicatorRun>,
    pub top_identifier: Option<MetricIdentifierVisual>,
    pub bottom_identifier: Option<MetricIdentifierVisual>,
    pub disk_badge: Option<IndicatorRun>,
    pub accessibility_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricIdentifierVisual {
    SystemSymbol {
        name: SystemSymbolName,
        color: SegmentColor,
        fallback_text: String,
    },
    Png {
        metric: MetricKind,
        metadata: PngIconMetadata,
        fallback_color: SegmentColor,
        fallback_text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricSurfaceSpec {
    pub identifier: Option<MetricIdentifierVisual>,
    pub runs: Vec<IndicatorRun>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceSpec {
    pub top: MetricSurfaceSpec,
    pub bottom: MetricSurfaceSpec,
    pub disk_badge: Option<IndicatorRun>,
}

impl IndicatorScene {
    pub fn surface_spec(&self) -> SurfaceSpec {
        SurfaceSpec {
            top: MetricSurfaceSpec {
                identifier: self.top_identifier.clone(),
                runs: self.top.clone(),
            },
            bottom: MetricSurfaceSpec {
                identifier: self.bottom_identifier.clone(),
                runs: self.bottom.clone(),
            },
            disk_badge: self.disk_badge.clone(),
        }
    }
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

    let (top_identifier, top) = metric_presentation(
        &status.cpu,
        MetricKind::Cpu,
        cpu_color,
        preferences,
        preferences.labels.cpu.as_str(),
        appearance,
    );
    let (bottom_identifier, bottom) = metric_presentation(
        &status.ram,
        MetricKind::Ram,
        ram_color,
        preferences,
        preferences.labels.ram.as_str(),
        appearance,
    );

    IndicatorScene {
        top,
        bottom,
        top_identifier,
        bottom_identifier,
        disk_badge: status.disk_badge.map(|badge| match badge {
            DiskBadge::Warning => IndicatorRun {
                text: " !".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::DiskWarning),
                trailing_spacing_level: 0,
            },
            DiskBadge::Error => IndicatorRun {
                text: " ×".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::DiskError),
                trailing_spacing_level: 0,
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
    preview_accessibility_summary_with_fallbacks(scene, resolved_colors, appearance, [false, false])
}

pub fn preview_accessibility_summary_with_fallbacks(
    scene: &IndicatorScene,
    resolved_colors: &[[f64; 3]],
    appearance: IndicatorAppearance,
    text_fallbacks: [bool; 2],
) -> String {
    let mut color_index = 0;
    let (cpu, cpu_has_identifier) = preview_metric_summary(
        "CPU",
        &scene.top,
        scene.top_identifier.as_ref(),
        resolved_colors,
        &mut color_index,
        text_fallbacks[0],
    );
    let (ram, ram_has_identifier) = preview_metric_summary(
        "RAM",
        &scene.bottom,
        scene.bottom_identifier.as_ref(),
        resolved_colors,
        &mut color_index,
        text_fallbacks[1],
    );
    let uses_visual_identifier =
        scene.top_identifier.is_some() || scene.bottom_identifier.is_some();
    let labels = if uses_visual_identifier && cpu_has_identifier && ram_has_identifier {
        "identificadores visíveis"
    } else if uses_visual_identifier {
        "identificadores parcialmente visíveis"
    } else if cpu_has_identifier && ram_has_identifier {
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

pub fn resolve_identifier_fallbacks(
    scene: &IndicatorScene,
    identifier_resolved: [bool; 2],
) -> (IndicatorScene, [bool; 2]) {
    let mut resolved = scene.clone();
    let top_fallback = resolve_metric_identifier_fallback(
        &mut resolved.top_identifier,
        &mut resolved.top,
        identifier_resolved[0],
    );
    let bottom_fallback = resolve_metric_identifier_fallback(
        &mut resolved.bottom_identifier,
        &mut resolved.bottom,
        identifier_resolved[1],
    );
    (resolved, [top_fallback, bottom_fallback])
}

fn resolve_metric_identifier_fallback(
    identifier: &mut Option<MetricIdentifierVisual>,
    runs: &mut Vec<IndicatorRun>,
    resolved: bool,
) -> bool {
    if resolved {
        return false;
    }
    let Some(identifier) = identifier.take() else {
        return false;
    };
    let (text, color) = match identifier {
        MetricIdentifierVisual::SystemSymbol {
            fallback_text,
            color,
            ..
        } => (fallback_text, color),
        MetricIdentifierVisual::Png {
            fallback_text,
            fallback_color,
            ..
        } => (fallback_text, fallback_color),
    };
    runs.insert(
        0,
        IndicatorRun {
            text,
            color,
            trailing_spacing_level: 0,
        },
    );
    true
}

pub fn preview_visible_summary(
    scene: &IndicatorScene,
    appearance: IndicatorAppearance,
    text_fallbacks: [bool; 2],
) -> String {
    let appearance = match appearance {
        IndicatorAppearance::Light => "Claro",
        IndicatorAppearance::Dark => "Escuro",
    };
    let cpu = visible_metric_summary(
        "CPU",
        &scene.top,
        scene.top_identifier.as_ref(),
        text_fallbacks[0],
    );
    let ram = visible_metric_summary(
        "RAM",
        &scene.bottom,
        scene.bottom_identifier.as_ref(),
        text_fallbacks[1],
    );
    format!("{appearance}: {cpu} · {ram}.")
}

fn visible_metric_summary(
    name: &str,
    runs: &[IndicatorRun],
    identifier: Option<&MetricIdentifierVisual>,
    text_fallback: bool,
) -> String {
    let value = runs.last().map_or("—", |run| run.text.trim());
    let presentation = if text_fallback {
        "texto alternativo"
    } else {
        match identifier {
            Some(MetricIdentifierVisual::SystemSymbol { .. }) => "ícone",
            Some(MetricIdentifierVisual::Png { .. }) => "PNG",
            None if runs.len() > 1 => "texto",
            None => "sem rótulo",
        }
    };
    format!("{name} {value} ({presentation})")
}

fn preview_metric_summary(
    name: &str,
    runs: &[IndicatorRun],
    identifier: Option<&MetricIdentifierVisual>,
    resolved_colors: &[[f64; 3]],
    color_index: &mut usize,
    text_fallback: bool,
) -> (String, bool) {
    if let (Some(identifier), [value]) = (identifier, runs) {
        return match identifier {
            MetricIdentifierVisual::SystemSymbol { name: symbol, .. } => {
                let identifier_color = resolved_colors
                    .get(*color_index)
                    .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
                *color_index += 1;
                let value_color = resolved_colors
                    .get(*color_index)
                    .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
                *color_index += 1;
                (
                    format!(
                        "{name} {}, ícone do macOS {} na cor {identifier_color} e valor {value_color}",
                        value.text.trim(),
                        symbol.label_pt_br()
                    ),
                    true,
                )
            }
            MetricIdentifierVisual::Png { metadata, .. } => {
                let value_color = resolved_colors
                    .get(*color_index)
                    .map_or_else(|| "cor indisponível".to_owned(), |color| color_hex(*color));
                *color_index += 1;
                (
                    format!(
                        "{name} {}, PNG {} e valor {value_color}",
                        value.text.trim(),
                        metadata.source_name()
                    ),
                    true,
                )
            }
        };
    }
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
                if text_fallback {
                    format!(
                        "{name} {}, fallback textual na cor {label_color} e valor {value_color}",
                        value.text.trim()
                    )
                } else {
                    format!(
                        "{name} {}, rótulo {label_color} e valor {value_color}",
                        value.text.trim()
                    )
                },
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
    label: &str,
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
            text: label.to_owned(),
            color,
            trailing_spacing_level: preferences.labels.spacing.level(),
        });
    }
    runs.push(IndicatorRun {
        text: format!("{}%", metric.percent),
        color: metric_color,
        trailing_spacing_level: 0,
    });
    runs
}

fn metric_presentation(
    metric: &MetricContent,
    metric_kind: MetricKind,
    metric_color: SegmentColor,
    preferences: &IndicatorPreferences,
    label: &str,
    appearance: IndicatorAppearance,
) -> (Option<MetricIdentifierVisual>, Vec<IndicatorRun>) {
    let identifier = match metric_kind {
        MetricKind::Cpu => &preferences.identifiers.cpu,
        MetricKind::Ram => &preferences.identifiers.ram,
    };
    if identifier.mode == MetricIdentifierMode::Text {
        return (
            None,
            metric_runs(metric, metric_color, preferences, label, appearance),
        );
    }

    let label_color = resolved_label_color(metric_color, preferences, appearance);
    let fallback_text = format!("{} ", metric.label);
    let value_run = IndicatorRun {
        text: format!("{}%", metric.percent),
        color: metric_color,
        trailing_spacing_level: 0,
    };
    match identifier.mode {
        MetricIdentifierMode::Text => unreachable!("text mode returned above"),
        MetricIdentifierMode::SystemSymbol => (
            Some(MetricIdentifierVisual::SystemSymbol {
                name: identifier.system_symbol.clone(),
                color: label_color,
                fallback_text,
            }),
            vec![value_run],
        ),
        MetricIdentifierMode::Png => match &identifier.png {
            Some(metadata) => (
                Some(MetricIdentifierVisual::Png {
                    metric: metric_kind,
                    metadata: metadata.clone(),
                    fallback_color: label_color,
                    fallback_text,
                }),
                vec![value_run],
            ),
            None => (
                None,
                vec![
                    IndicatorRun {
                        text: fallback_text,
                        color: label_color,
                        trailing_spacing_level: 0,
                    },
                    value_run,
                ],
            ),
        },
    }
}

fn resolved_label_color(
    metric_color: SegmentColor,
    preferences: &IndicatorPreferences,
    appearance: IndicatorAppearance,
) -> SegmentColor {
    match preferences.labels.color_mode {
        LabelColorMode::Neutral => SegmentColor::Semantic(SemanticColor::Neutral),
        LabelColorMode::MatchMetric => metric_color,
        LabelColorMode::Fixed => SegmentColor::Srgb(preferences.labels.fixed.color_for(appearance)),
    }
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
    let (cpu_prefix, ram_prefix) = if labels_visible {
        (Some("C "), Some("R "))
    } else {
        (None, None)
    };
    measure_stable_layout_with_prefixes(measurer, cpu_prefix, ram_prefix, default_width)
}

pub fn measure_stable_layout_with_prefixes(
    measurer: &impl TextMeasurer,
    cpu_prefix: Option<&str>,
    ram_prefix: Option<&str>,
    default_width: f64,
) -> StableLayout {
    let cpu_width = widest_metric_width(measurer, cpu_prefix);
    let ram_width = widest_metric_width(measurer, ram_prefix);
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

pub fn measure_stable_layout_with_prefixes_and_spacing(
    measurer: &impl TextMeasurer,
    cpu_prefix: Option<&str>,
    ram_prefix: Option<&str>,
    cpu_spacing_level: u8,
    ram_spacing_level: u8,
    default_width: f64,
) -> StableLayout {
    let cpu_width = widest_metric_width_with_spacing(measurer, cpu_prefix, cpu_spacing_level);
    let ram_width = widest_metric_width_with_spacing(measurer, ram_prefix, ram_spacing_level);
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

fn widest_metric_width(measurer: &impl TextMeasurer, prefix: Option<&str>) -> f64 {
    (0..=100)
        .map(|percent| {
            let text = format!("{}{}%", prefix.unwrap_or(""), percent);
            measurer.width(&text)
        })
        .fold(0.0, f64::max)
}

fn widest_metric_width_with_spacing(
    measurer: &impl TextMeasurer,
    prefix: Option<&str>,
    spacing_level: u8,
) -> f64 {
    let prefix_width = prefix.map_or(0.0, |prefix| measurer.width(prefix));
    let spacing_width = measurer.width(" ") * f64::from(spacing_level) / 10.0;
    (0..=100)
        .map(|percent| measurer.width(&format!("{percent}%")) + prefix_width + spacing_width)
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
