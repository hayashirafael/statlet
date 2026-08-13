use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSAccessibility, NSColor, NSFont, NSImageScaling, NSImageView, NSLineBreakMode, NSScrollView,
    NSStackView, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use statlet::indicator::{LayoutDiagnostics, PreviewBackground};
use statlet::indicator_preferences::FontFamilyPreference;

use super::super::{IndicatorFontFallback, PreviewContrastWarnings, PreviewSummaries};
use crate::macos::environment::VisualEnvironment;
use crate::macos::renderer::PreviewImages;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreviewFallback<'a> {
    requested_family: &'a str,
    resolved_family: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreviewText {
    light_description: String,
    dark_description: String,
    light_accessibility: String,
    dark_accessibility: String,
    shared_warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WarningRegionContract {
    maximum_number_of_lines: isize,
    scrollable: bool,
}

const fn warning_region_contract() -> WarningRegionContract {
    WarningRegionContract {
        maximum_number_of_lines: 0,
        scrollable: true,
    }
}

fn preview_text(
    summaries: &PreviewSummaries,
    layout: &LayoutDiagnostics,
    fallback: Option<PreviewFallback<'_>>,
    environment: &VisualEnvironment,
    contrast: PreviewContrastWarnings,
) -> PreviewText {
    let mut shared_warnings = Vec::new();
    match (layout.exceeds_menu_bar_height, layout.exceeds_curated_width) {
        (true, true) => shared_warnings.push(
            "A tipografia pode cortar as linhas na altura e ocupar largura excessiva.".to_owned(),
        ),
        (true, false) => shared_warnings
            .push("A tipografia pode cortar as linhas na altura da menu bar.".to_owned()),
        (false, true) => shared_warnings
            .push("A tipografia pode ocupar largura excessiva na menu bar.".to_owned()),
        (false, false) => {}
    }
    if let Some(fallback) = fallback {
        shared_warnings.push(format!(
            "A fonte {} não está disponível; usando {} sem alterar sua escolha.",
            fallback.requested_family, fallback.resolved_family
        ));
    }
    if environment.increase_contrast {
        shared_warnings.push(
            "Aumentar Contraste está ativo; as cores semânticas das prévias usam as aparências de alto contraste do macOS."
                .to_owned(),
        );
    }
    if environment.differentiate_without_color {
        shared_warnings.push(
            "Diferenciar Sem Cor está ativo; valores e badges com símbolos permanecem visíveis."
                .to_owned(),
        );
    }
    if environment.reduce_transparency {
        shared_warnings.push(
            "Reduzir Transparência está ativo; os fundos representativos usam preenchimento opaco."
                .to_owned(),
        );
    }
    if contrast.light {
        shared_warnings.push("Prévia clara: o contraste pode ficar abaixo de 4,5:1.".to_owned());
    }
    if contrast.dark {
        shared_warnings.push("Prévia escura: o contraste pode ficar abaixo de 4,5:1.".to_owned());
    }
    shared_warnings.push(
        "As prévias não reproduzem papel de parede, transparência nem todo estado real da menu bar."
            .to_owned(),
    );

    PreviewText {
        light_description: summaries.light_visible.clone(),
        dark_description: summaries.dark_visible.clone(),
        light_accessibility: summaries.light.clone(),
        dark_accessibility: summaries.dark.clone(),
        shared_warnings,
    }
}

pub(super) struct PreviewPane {
    view: Retained<NSStackView>,
    light_background: Retained<NSTextField>,
    dark_background: Retained<NSTextField>,
    light_image: Retained<NSImageView>,
    dark_image: Retained<NSImageView>,
    light_description: Retained<NSTextField>,
    dark_description: Retained<NSTextField>,
    warnings: Retained<NSTextField>,
    warnings_scroll: Retained<NSScrollView>,
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
        let light_description = wrapped_preview_label(
            mtm,
            "Prévia clara do indicador em escala aproximada da menu bar.",
            NSRect::new(NSPoint::new(0.0, 56.0), NSSize::new(300.0, 32.0)),
        );
        light_description.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            accessibility_identifiers[0],
        )));
        let dark_description = wrapped_preview_label(
            mtm,
            "Prévia escura do indicador em escala aproximada da menu bar.",
            NSRect::new(NSPoint::new(316.0, 56.0), NSSize::new(300.0, 32.0)),
        );
        dark_description.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            accessibility_identifiers[1],
        )));
        let warnings_scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(616.0, 52.0)),
        );
        let warning_contract = warning_region_contract();
        warnings_scroll.setHasVerticalScroller(warning_contract.scrollable);
        warnings_scroll.setAutohidesScrollers(true);
        warnings_scroll.setDrawsBackground(false);
        let warnings = wrapped_preview_label(
            mtm,
            "• As prévias não reproduzem papel de parede, transparência nem todo estado real da menu bar.",
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(598.0, 52.0)),
        );
        warnings.setMaximumNumberOfLines(warning_contract.maximum_number_of_lines);
        warnings.setTextColor(Some(&NSColor::secondaryLabelColor()));
        warnings.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            "indicator.preview.warnings",
        )));
        warnings_scroll.setDocumentView(Some(&warnings));

        for child in [
            &*light_background as &NSView,
            &*dark_background,
            &*light_heading,
            &*dark_heading,
            &*light_image,
            &*dark_image,
            &*light_description,
            &*dark_description,
            &*warnings_scroll,
        ] {
            view.addSubview(child);
        }

        Self {
            view,
            light_background,
            dark_background,
            light_image,
            dark_image,
            light_description,
            dark_description,
            warnings,
            warnings_scroll,
        }
    }

    pub(super) fn view(&self) -> &NSStackView {
        &self.view
    }

    pub(super) fn apply_with_contrast(
        &self,
        images: &PreviewImages,
        layout: &LayoutDiagnostics,
        fallback: Option<&IndicatorFontFallback>,
        environment: &VisualEnvironment,
        contrast: PreviewContrastWarnings,
        summaries: &PreviewSummaries,
    ) {
        let fallback = fallback.map(|fallback| PreviewFallback {
            requested_family: family_name(&fallback.requested_family),
            resolved_family: &fallback.resolved_family,
        });
        self.apply_text(images, layout, fallback, environment, contrast, summaries);
    }

    fn apply_text(
        &self,
        images: &PreviewImages,
        layout: &LayoutDiagnostics,
        fallback: Option<PreviewFallback<'_>>,
        environment: &VisualEnvironment,
        contrast: PreviewContrastWarnings,
        summaries: &PreviewSummaries,
    ) {
        let preview_plan = environment.preview_plan();
        apply_preview_background(
            &self.light_background,
            PreviewBackground::Light,
            preview_plan.background_opacity,
        );
        apply_preview_background(
            &self.dark_background,
            PreviewBackground::Dark,
            preview_plan.background_opacity,
        );
        self.light_image.setImage(Some(&images.light));
        self.dark_image.setImage(Some(&images.dark));
        let text = preview_text(summaries, layout, fallback, environment, contrast);
        set_preview_text(
            &self.light_description,
            &text.light_description,
            &text.light_accessibility,
        );
        set_preview_text(
            &self.dark_description,
            &text.dark_description,
            &text.dark_accessibility,
        );
        set_warning_text(&self.warnings, &self.warnings_scroll, &text.shared_warnings);
    }
}

fn preview_background(
    mtm: MainThreadMarker,
    frame: NSRect,
    background: PreviewBackground,
) -> Retained<NSTextField> {
    let field = preview_label(mtm, "", frame);
    field.setDrawsBackground(true);
    apply_preview_background(&field, background, 0.82);
    field.setAccessibilityElement(false);
    field
}

fn apply_preview_background(field: &NSTextField, background: PreviewBackground, opacity: f64) {
    let [red, green, blue] = background.components();
    field.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        red, green, blue, opacity,
    )));
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

fn wrapped_preview_label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
) -> Retained<NSTextField> {
    let field = preview_label(mtm, text, frame);
    field.setUsesSingleLineMode(false);
    field.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    field.setMaximumNumberOfLines(0);
    field
}

fn set_preview_text(field: &NSTextField, visible: &str, accessibility: &str) {
    field.setStringValue(&objc2_foundation::NSString::from_str(visible));
    field.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(accessibility)));
}

fn set_warning_text(field: &NSTextField, scroll: &NSScrollView, warnings: &[String]) {
    let text = warnings
        .iter()
        .map(|warning| format!("• {warning}"))
        .collect::<Vec<_>>()
        .join("\n");
    set_preview_text(field, &text, &text);
    let viewport = scroll.contentSize();
    let measured = field.sizeThatFits(NSSize::new(viewport.width, f64::MAX));
    field.setFrameSize(NSSize::new(
        viewport.width,
        measured.height.max(viewport.height),
    ));
}

fn family_name(family: &FontFamilyPreference) -> &str {
    match family {
        FontFamilyPreference::SystemMonospaced => "System Monospaced",
        FontFamilyPreference::Named(family) => family,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        preview_text, warning_region_contract, PreviewContrastWarnings, PreviewFallback,
        PreviewSummaries,
    };
    use crate::macos::environment::VisualEnvironment;
    use statlet::indicator::LayoutDiagnostics;
    use statlet::indicator_preferences::IndicatorAppearance;

    #[test]
    fn preview_text_keeps_appearance_descriptions_separate_from_shared_warnings() {
        let summaries = PreviewSummaries {
            light_visible: "Claro: CPU 42% · RAM 68%.".into(),
            dark_visible: "Escuro: CPU 42% · RAM 68%.".into(),
            light: "Prévia clara resumida e completa para acessibilidade.".into(),
            dark: "Prévia escura resumida e completa para acessibilidade.".into(),
        };
        let text = preview_text(
            &summaries,
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

        assert!(text.light_description.contains("Claro"));
        assert!(text.light_accessibility.contains("Prévia clara"));
        assert!(!text.light_description.contains("contraste"));
        assert!(!text.light_description.contains("Prévia escura"));
        assert!(text.dark_description.contains("Escuro"));
        assert!(text.dark_accessibility.contains("Prévia escura"));
        assert!(!text.dark_description.contains("contraste"));
        assert!(!text.dark_description.contains("Prévia clara"));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("Prévia clara") && warning.contains("4,5:1")));
        assert!(!text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("Prévia escura") && warning.contains("4,5:1")));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("altura") && warning.contains("largura")));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("Fonte ausente")));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("Aumentar Contraste")));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("símbolos")));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("Reduzir Transparência")));
        assert!(text
            .shared_warnings
            .iter()
            .any(|warning| warning.contains("papel de parede") && warning.contains("estado real")));
    }

    #[test]
    fn preview_text_routes_each_appearance_contrast_warning_once_to_scrollable_region() {
        let summaries = PreviewSummaries {
            light_visible: "Claro: CPU 42% · RAM 68%.".into(),
            dark_visible: "Escuro: CPU 42% · RAM 68%.".into(),
            light: "Prévia clara resumida.".into(),
            dark: "Prévia escura resumida.".into(),
        };
        let text = preview_text(
            &summaries,
            &LayoutDiagnostics {
                exceeds_menu_bar_height: false,
                exceeds_curated_width: false,
            },
            None,
            &VisualEnvironment {
                appearance: IndicatorAppearance::Dark,
                increase_contrast: false,
                differentiate_without_color: false,
                reduce_transparency: false,
            },
            PreviewContrastWarnings {
                light: true,
                dark: true,
            },
        );

        assert!(!text.light_description.contains("Aviso:"));
        assert!(!text.dark_description.contains("Aviso:"));
        assert_eq!(
            text.shared_warnings
                .iter()
                .filter(|warning| warning.contains("Prévia clara") && warning.contains("4,5:1"))
                .count(),
            1
        );
        assert_eq!(
            text.shared_warnings
                .iter()
                .filter(|warning| warning.contains("Prévia escura") && warning.contains("4,5:1"))
                .count(),
            1
        );
    }

    #[test]
    fn warning_region_is_scrollable_and_never_limits_visible_lines() {
        let contract = warning_region_contract();

        assert_eq!(contract.maximum_number_of_lines, 0);
        assert!(contract.scrollable);
    }
}
