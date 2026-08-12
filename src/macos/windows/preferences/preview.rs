use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSAccessibility, NSColor, NSFont, NSImageScaling, NSImageView, NSStackView, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use statlet::indicator::LayoutDiagnostics;
use statlet::indicator_preferences::{FontFamilyPreference, SrgbColor};

use super::super::{IndicatorFontFallback, PreviewContrastWarnings};
use crate::macos::environment::VisualEnvironment;
use crate::macos::fonts::FontResolution;
use crate::macos::renderer::PreviewImages;

const SMALL_TEXT_CONTRAST_THRESHOLD: f64 = 4.5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewBackground {
    Light,
    Dark,
}

impl PreviewBackground {
    const fn components(self) -> [f64; 3] {
        match self {
            Self::Light => [1.0, 1.0, 1.0],
            Self::Dark => [30.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreviewFallback<'a> {
    requested_family: &'a str,
    resolved_family: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreviewText {
    summary: String,
    warnings: String,
}

fn preview_text(
    layout: &LayoutDiagnostics,
    fallback: Option<PreviewFallback<'_>>,
    environment: &VisualEnvironment,
    contrast: PreviewContrastWarnings,
) -> PreviewText {
    let mut warnings = Vec::new();
    if contrast.light {
        warnings.push("O contraste da prévia clara pode ficar abaixo de 4,5:1.".to_owned());
    }
    if contrast.dark {
        warnings.push("O contraste da prévia escura pode ficar abaixo de 4,5:1.".to_owned());
    }
    match (layout.exceeds_menu_bar_height, layout.exceeds_curated_width) {
        (true, true) => warnings.push(
            "A tipografia pode cortar as linhas na altura e ocupar largura excessiva.".to_owned(),
        ),
        (true, false) => {
            warnings.push("A tipografia pode cortar as linhas na altura da menu bar.".to_owned())
        }
        (false, true) => {
            warnings.push("A tipografia pode ocupar largura excessiva na menu bar.".to_owned())
        }
        (false, false) => {}
    }
    if let Some(fallback) = fallback {
        warnings.push(format!(
            "A fonte {} não está disponível; usando {} sem alterar sua escolha.",
            fallback.requested_family, fallback.resolved_family
        ));
    }
    if environment.increase_contrast {
        warnings.push(
            "Aumentar Contraste está ativo; as cores semânticas foram resolvidas novamente."
                .to_owned(),
        );
    }
    if environment.differentiate_without_color {
        warnings.push(
            "Diferenciar Sem Cor está ativo; valores e badges com símbolos permanecem visíveis."
                .to_owned(),
        );
    }
    if environment.reduce_transparency {
        warnings.push(
            "Reduzir Transparência está ativo; o fundo representativo da prévia foi atualizado."
                .to_owned(),
        );
    }
    warnings.push(
        "As prévias não reproduzem papel de parede, transparência nem todo estado real da menu bar."
            .to_owned(),
    );

    PreviewText {
        summary: "Prévias Claro e Escuro do indicador em escala aproximada da menu bar.".to_owned(),
        warnings: warnings.join(" "),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn contrast_warning(
    foreground: SrgbColor,
    background: PreviewBackground,
) -> Option<f64> {
    let foreground = foreground
        .components()
        .map(|component| f64::from(component) / 255.0);
    let ratio = contrast_ratio(foreground, background.components());
    (ratio < SMALL_TEXT_CONTRAST_THRESHOLD).then_some(ratio)
}

fn contrast_ratio(left: [f64; 3], right: [f64; 3]) -> f64 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
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

pub(super) struct PreviewPane {
    view: Retained<NSStackView>,
    light_image: Retained<NSImageView>,
    dark_image: Retained<NSImageView>,
    summary: Retained<NSTextField>,
    warnings: Retained<NSTextField>,
}

impl PreviewPane {
    pub(super) fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        accessibility_identifiers: [&str; 2],
    ) -> Self {
        let view = NSStackView::initWithFrame(NSStackView::alloc(mtm), frame);
        let light_background = preview_background(
            mtm,
            NSRect::new(NSPoint::new(0.0, 90.0), NSSize::new(300.0, 30.0)),
            PreviewBackground::Light,
        );
        let dark_background = preview_background(
            mtm,
            NSRect::new(NSPoint::new(316.0, 90.0), NSSize::new(300.0, 30.0)),
            PreviewBackground::Dark,
        );
        let light_heading = preview_heading(mtm, "Claro", 0.0);
        let dark_heading = preview_heading(mtm, "Escuro", 316.0);
        let light_image = preview_image(
            mtm,
            NSRect::new(NSPoint::new(4.0, 94.0), NSSize::new(292.0, 22.0)),
        );
        let dark_image = preview_image(
            mtm,
            NSRect::new(NSPoint::new(320.0, 94.0), NSSize::new(292.0, 22.0)),
        );
        let summary = preview_label(
            mtm,
            "Prévias Claro e Escuro do indicador em escala aproximada da menu bar.",
            NSRect::new(NSPoint::new(0.0, 58.0), NSSize::new(616.0, 20.0)),
        );
        summary.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            accessibility_identifiers[0],
        )));
        let warnings = preview_label(
            mtm,
            "As prévias não reproduzem papel de parede, transparência nem todo estado real da menu bar.",
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(616.0, 54.0)),
        );
        warnings.setMaximumNumberOfLines(3);
        warnings.setTextColor(Some(&NSColor::secondaryLabelColor()));
        warnings.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            accessibility_identifiers[1],
        )));

        for child in [
            &*light_background as &NSView,
            &*dark_background,
            &*light_heading,
            &*dark_heading,
            &*light_image,
            &*dark_image,
            &*summary,
            &*warnings,
        ] {
            view.addSubview(child);
        }

        Self {
            view,
            light_image,
            dark_image,
            summary,
            warnings,
        }
    }

    pub(super) fn view(&self) -> &NSStackView {
        &self.view
    }

    #[allow(dead_code)]
    pub(super) fn apply(
        &self,
        images: &PreviewImages,
        layout: &LayoutDiagnostics,
        fallback: Option<&FontResolution>,
        environment: &VisualEnvironment,
    ) {
        let fallback = fallback.map(|fallback| PreviewFallback {
            requested_family: family_name(&fallback.requested_family),
            resolved_family: &fallback.resolved_family,
        });
        self.apply_text(
            images,
            layout,
            fallback,
            environment,
            PreviewContrastWarnings::default(),
        );
    }

    pub(super) fn apply_with_contrast(
        &self,
        images: &PreviewImages,
        layout: &LayoutDiagnostics,
        fallback: Option<&IndicatorFontFallback>,
        environment: &VisualEnvironment,
        contrast: PreviewContrastWarnings,
    ) {
        let fallback = fallback.map(|fallback| PreviewFallback {
            requested_family: family_name(&fallback.requested_family),
            resolved_family: &fallback.resolved_family,
        });
        self.apply_text(images, layout, fallback, environment, contrast);
    }

    fn apply_text(
        &self,
        images: &PreviewImages,
        layout: &LayoutDiagnostics,
        fallback: Option<PreviewFallback<'_>>,
        environment: &VisualEnvironment,
        contrast: PreviewContrastWarnings,
    ) {
        self.light_image.setImage(Some(&images.light));
        self.dark_image.setImage(Some(&images.dark));
        let text = preview_text(layout, fallback, environment, contrast);
        set_preview_text(&self.summary, &text.summary);
        set_preview_text(&self.warnings, &text.warnings);
    }
}

fn preview_background(
    mtm: MainThreadMarker,
    frame: NSRect,
    background: PreviewBackground,
) -> Retained<NSTextField> {
    let field = preview_label(mtm, "", frame);
    let [red, green, blue] = background.components();
    field.setDrawsBackground(true);
    field.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        red, green, blue, 1.0,
    )));
    field.setAccessibilityElement(false);
    field
}

fn preview_heading(mtm: MainThreadMarker, title: &str, x: f64) -> Retained<NSTextField> {
    let field = preview_label(
        mtm,
        title,
        NSRect::new(NSPoint::new(x, 124.0), NSSize::new(300.0, 20.0)),
    );
    field.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    field
}

fn preview_image(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSImageView> {
    let image = NSImageView::initWithFrame(NSImageView::alloc(mtm), frame);
    image.setImageScaling(NSImageScaling::ScaleNone);
    image.setAccessibilityElement(false);
    image
}

fn preview_label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(text), mtm);
    field.setFrame(frame);
    field
}

fn set_preview_text(field: &NSTextField, text: &str) {
    let text = objc2_foundation::NSString::from_str(text);
    field.setStringValue(&text);
    field.setAccessibilityLabel(Some(&text));
}

fn family_name(family: &FontFamilyPreference) -> &str {
    match family {
        FontFamilyPreference::SystemMonospaced => "System Monospaced",
        FontFamilyPreference::Named(family) => family,
    }
}

#[cfg(test)]
mod tests {
    use statlet::indicator::LayoutDiagnostics;
    use statlet::indicator_preferences::IndicatorAppearance;
    use statlet::indicator_preferences::SrgbColor;

    use super::{
        contrast_warning, preview_text, PreviewBackground, PreviewContrastWarnings, PreviewFallback,
    };
    use crate::macos::environment::VisualEnvironment;

    #[test]
    fn small_text_below_four_point_five_to_one_warns_without_replacing_color() {
        let chosen = SrgbColor::parse_hex("#777777").unwrap();

        let warning = contrast_warning(chosen, PreviewBackground::Light);

        assert!(warning.is_some());
        assert_eq!(chosen.to_hex(), "#777777");
    }

    #[test]
    fn high_contrast_small_text_on_the_dark_preview_does_not_warn() {
        let chosen = SrgbColor::parse_hex("#FFFFFF").unwrap();

        let warning = contrast_warning(chosen, PreviewBackground::Dark);

        assert!(warning.is_none());
        assert_eq!(chosen.to_hex(), "#FFFFFF");
    }

    #[test]
    fn preview_text_exposes_accessibility_states_and_real_world_limitations() {
        let text = preview_text(
            &LayoutDiagnostics {
                exceeds_menu_bar_height: true,
                exceeds_curated_width: true,
            },
            Some(PreviewFallback {
                requested_family: "Fonte ausente",
                resolved_family: "SF Mono",
            }),
            &VisualEnvironment {
                appearance: IndicatorAppearance::Dark,
                increase_contrast: true,
                differentiate_without_color: true,
                reduce_transparency: true,
            },
            PreviewContrastWarnings {
                light: true,
                dark: false,
            },
        );

        assert!(text.summary.contains("Claro e Escuro"));
        assert!(text.warnings.contains("contraste da prévia clara"));
        assert!(text.warnings.contains("altura"));
        assert!(text.warnings.contains("largura"));
        assert!(text.warnings.contains("Fonte ausente"));
        assert!(text.warnings.contains("Aumentar Contraste"));
        assert!(text.warnings.contains("símbolos"));
        assert!(text.warnings.contains("Reduzir Transparência"));
        assert!(text.warnings.contains("papel de parede"));
        assert!(text.warnings.contains("estado real"));
    }
}
