//! Two-line status-item renderer.
//!
//! Derived and modified from featherbar commit 90ab504, Apache-2.0:
//! https://github.com/nim444/featherbar/tree/90ab504b025db15665ce5d97b8ae4d4cdeb47dc3

use std::cell::RefCell;

use block2::StackBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{
    NSAccessibility, NSAppearance, NSApplication, NSAttributedStringNSStringDrawing, NSColor,
    NSColorSpace, NSFont, NSFontAttributeName, NSFontWeightBold, NSFontWeightMedium,
    NSFontWeightRegular, NSForegroundColorAttributeName, NSImage, NSImageSymbolConfiguration,
    NSStatusBarButton, NSView,
};
use objc2_foundation::{
    NSDictionary, NSMutableAttributedString, NSPoint, NSRect, NSSize, NSString,
};

use statlet::icon_assets::IconAssetStore;
use statlet::indicator::{
    measure_stable_layout, measure_stable_layout_with_prefixes_and_spacing, trailing_spacing_width,
    IndicatorRun, IndicatorScene, LayoutDiagnostics, MetricIdentifierVisual, SegmentColor,
    SemanticColor, StableLayout, TextMeasurer,
};
use statlet::indicator_preferences::{
    FontWeight, MetricKind, PngIconMetadata, TypographyPreferences,
};
use statlet::runtime_profile::RuntimePresentation;

use super::fonts::{FontCatalog, FontResolution};

const LINE_GAP: f64 = 2.0;
const HEIGHT: f64 = 22.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderSlot {
    Status,
    PreviewLight,
    PreviewDark,
}

impl RenderSlot {
    const fn index(self) -> usize {
        match self {
            Self::Status => 0,
            Self::PreviewLight => 1,
            Self::PreviewDark => 2,
        }
    }
}

struct SlotMap<T> {
    entries: [Option<T>; 3],
}

impl<T> Default for SlotMap<T> {
    fn default() -> Self {
        Self {
            entries: [None, None, None],
        }
    }
}

impl<T> SlotMap<T> {
    fn replace(&mut self, slot: RenderSlot, value: T) {
        self.entries[slot.index()] = Some(value);
    }

    fn get(&self, slot: RenderSlot) -> Option<&T> {
        self.entries[slot.index()].as_ref()
    }

    fn clear(&mut self) {
        self.entries = [None, None, None];
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutKey {
    resolved_family: String,
    size: u8,
    weight: FontWeight,
    cpu_prefix: Option<String>,
    ram_prefix: Option<String>,
    cpu_spacing_level: u8,
    ram_spacing_level: u8,
    dev_marker: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaintIdentity {
    Srgb([u8; 3]),
    Semantic {
        color: statlet::indicator::SemanticColor,
        appearance: String,
    },
}

fn paint_identity(color: SegmentColor, appearance: &str) -> PaintIdentity {
    match color_plan(color) {
        ColorPlan::Srgb(components) => PaintIdentity::Srgb(components),
        ColorPlan::Semantic(color) => PaintIdentity::Semantic {
            color,
            appearance: appearance.to_owned(),
        },
    }
}

pub(crate) trait StatusMetadataTarget {
    fn set_accessibility_label(&self, value: &str);
    fn set_tooltip(&self, value: &str);
}

pub(crate) trait StatusRenderTarget: StatusMetadataTarget {
    fn set_status_image(&self, image: &NSImage);
}

impl StatusMetadataTarget for NSStatusBarButton {
    fn set_accessibility_label(&self, value: &str) {
        self.setAccessibilityLabel(Some(&NSString::from_str(value)));
    }

    fn set_tooltip(&self, value: &str) {
        self.setToolTip(Some(&NSString::from_str(value)));
    }
}

impl StatusRenderTarget for NSStatusBarButton {
    fn set_status_image(&self, image: &NSImage) {
        self.setImage(Some(image));
    }
}

fn apply_status_metadata(
    target: &impl StatusMetadataTarget,
    scene: &IndicatorScene,
    presentation: &RuntimePresentation,
) {
    let metadata = presentation.status_metadata(&scene.accessibility_label);
    target.set_accessibility_label(&metadata);
    target.set_tooltip(&metadata);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColorPlan {
    Semantic(SemanticColor),
    Srgb([u8; 3]),
}

fn color_plan(color: SegmentColor) -> ColorPlan {
    match color {
        SegmentColor::Semantic(color) => ColorPlan::Semantic(color),
        SegmentColor::Srgb(color) => ColorPlan::Srgb(color.components()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaintKey {
    layout: LayoutKey,
    top_identifier: Option<IdentifierIdentity>,
    bottom_identifier: Option<IdentifierIdentity>,
    top: Vec<RunIdentity>,
    bottom: Vec<RunIdentity>,
    badge: Option<RunIdentity>,
    dev_marker: Option<RunIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IdentifierIdentity {
    SystemSymbol {
        name: String,
        paint: PaintIdentity,
        fallback_text: String,
    },
    Png {
        metric: MetricKind,
        metadata: PngIconMetadata,
        fallback_paint: PaintIdentity,
        fallback_text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RunIdentity {
    text: String,
    paint: PaintIdentity,
    trailing_spacing_level: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaintDecision {
    Reuse,
    Repaint,
}

fn paint_decision(cached: Option<&PaintKey>, requested: &PaintKey) -> PaintDecision {
    match cached {
        Some(cached) if cached == requested => PaintDecision::Reuse,
        _ => PaintDecision::Repaint,
    }
}

fn paint_key(scene: &IndicatorScene, layout: LayoutKey, appearance: String) -> PaintKey {
    let dev_marker = layout.dev_marker.as_ref().map(|text| {
        run_identity(
            &IndicatorRun {
                text: text.clone(),
                color: SegmentColor::Semantic(SemanticColor::Neutral),
                trailing_spacing_level: 0,
            },
            &appearance,
        )
    });
    PaintKey {
        layout,
        top_identifier: scene
            .top_identifier
            .as_ref()
            .map(|identifier| identifier_identity(identifier, &appearance)),
        bottom_identifier: scene
            .bottom_identifier
            .as_ref()
            .map(|identifier| identifier_identity(identifier, &appearance)),
        top: run_identities(&scene.top, &appearance),
        bottom: run_identities(&scene.bottom, &appearance),
        badge: scene
            .disk_badge
            .as_ref()
            .map(|run| run_identity(run, &appearance)),
        dev_marker,
    }
}

impl PaintKey {
    fn uses_semantic_color(&self) -> bool {
        self.top_identifier
            .iter()
            .chain(self.bottom_identifier.iter())
            .any(|identifier| {
                matches!(
                    identifier,
                    IdentifierIdentity::SystemSymbol {
                        paint: PaintIdentity::Semantic { .. },
                        ..
                    } | IdentifierIdentity::Png {
                        fallback_paint: PaintIdentity::Semantic { .. },
                        ..
                    }
                )
            })
            || self
                .top
                .iter()
                .chain(&self.bottom)
                .chain(self.badge.iter())
                .chain(self.dev_marker.iter())
                .any(|run| matches!(run.paint, PaintIdentity::Semantic { .. }))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct StatusMarkerPlan {
    text: Option<String>,
    extra_width: f64,
}

fn status_marker_plan(
    presentation: &RuntimePresentation,
    measure: impl FnOnce(&str) -> f64,
) -> StatusMarkerPlan {
    let text = presentation.dev_marker().map(|marker| format!(" {marker}"));
    let extra_width = text.as_deref().map_or(0.0, measure);
    StatusMarkerPlan { text, extra_width }
}

fn identifier_identity(
    identifier: &MetricIdentifierVisual,
    appearance: &str,
) -> IdentifierIdentity {
    match identifier {
        MetricIdentifierVisual::SystemSymbol {
            name,
            color,
            fallback_text,
        } => IdentifierIdentity::SystemSymbol {
            name: name.as_str().to_owned(),
            paint: paint_identity(*color, appearance),
            fallback_text: fallback_text.clone(),
        },
        MetricIdentifierVisual::Png {
            metric,
            metadata,
            fallback_color,
            fallback_text,
        } => IdentifierIdentity::Png {
            metric: *metric,
            metadata: metadata.clone(),
            fallback_paint: paint_identity(*fallback_color, appearance),
            fallback_text: fallback_text.clone(),
        },
    }
}

fn run_identities(runs: &[IndicatorRun], appearance: &str) -> Vec<RunIdentity> {
    runs.iter()
        .map(|run| run_identity(run, appearance))
        .collect()
}

fn run_identity(run: &IndicatorRun, appearance: &str) -> RunIdentity {
    RunIdentity {
        text: run.text.clone(),
        paint: paint_identity(run.color, appearance),
        trailing_spacing_level: run.trailing_spacing_level,
    }
}

struct SlotCache<I> {
    layout_key: LayoutKey,
    layout: StableLayout,
    paint_key: PaintKey,
    image: I,
}

struct SurfaceCache<I> {
    slots: SlotMap<SlotCache<I>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IdentifierImageKey {
    identity: IdentifierImageIdentity,
    size: u8,
    weight: FontWeight,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IdentifierImageIdentity {
    SystemSymbol {
        name: String,
        paint: PaintIdentity,
    },
    Png {
        metric: MetricKind,
        metadata: PngIconMetadata,
    },
}

fn identifier_image_key(
    identifier: &MetricIdentifierVisual,
    typography: &TypographyPreferences,
    appearance: &str,
) -> IdentifierImageKey {
    let identity = match identifier {
        MetricIdentifierVisual::SystemSymbol { name, color, .. } => {
            IdentifierImageIdentity::SystemSymbol {
                name: name.as_str().to_owned(),
                paint: paint_identity(*color, appearance),
            }
        }
        MetricIdentifierVisual::Png {
            metric, metadata, ..
        } => IdentifierImageIdentity::Png {
            metric: *metric,
            metadata: metadata.clone(),
        },
    };
    IdentifierImageKey {
        identity,
        size: typography.size.points(),
        weight: typography.weight,
    }
}

struct IdentifierImageEntry<I> {
    key: IdentifierImageKey,
    image: Option<I>,
}

struct IdentifierImageCache<I> {
    entries: Vec<IdentifierImageEntry<I>>,
}

impl<I> Default for IdentifierImageCache<I> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<I: Clone> IdentifierImageCache<I> {
    fn resolve(&mut self, key: IdentifierImageKey, load: impl FnOnce() -> Option<I>) -> Option<I> {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let entry = self.entries.remove(index);
            let image = entry.image.clone();
            self.entries.push(entry);
            return image;
        }
        let image = load();
        const CAPACITY: usize = 12;
        if self.entries.len() == CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push(IdentifierImageEntry {
            key,
            image: image.clone(),
        });
        image
    }

    fn clear_semantic(&mut self) {
        self.entries.retain(|entry| {
            !matches!(
                entry.key.identity,
                IdentifierImageIdentity::SystemSymbol {
                    paint: PaintIdentity::Semantic { .. },
                    ..
                }
            )
        });
    }
}

impl<I> Default for SurfaceCache<I> {
    fn default() -> Self {
        Self {
            slots: SlotMap::default(),
        }
    }
}

impl<I: Clone> SurfaceCache<I> {
    fn resolve_layout(
        &self,
        slot: RenderSlot,
        requested: &LayoutKey,
        measure: impl FnOnce() -> StableLayout,
    ) -> StableLayout {
        match self.slots.get(slot) {
            Some(cached) if &cached.layout_key == requested => cached.layout,
            _ => measure(),
        }
    }

    fn reused_image(&self, slot: RenderSlot, requested: &PaintKey) -> Option<I> {
        self.slots.get(slot).and_then(|cached| {
            (paint_decision(Some(&cached.paint_key), requested) == PaintDecision::Reuse)
                .then(|| cached.image.clone())
        })
    }

    fn replace(
        &mut self,
        slot: RenderSlot,
        layout_key: LayoutKey,
        layout: StableLayout,
        paint_key: PaintKey,
        image: I,
    ) {
        self.slots.replace(
            slot,
            SlotCache {
                layout_key,
                layout,
                paint_key,
                image,
            },
        );
    }

    fn clear(&mut self) {
        self.slots.clear();
    }

    fn clear_semantic_paint(&mut self) {
        for slot in &mut self.slots.entries {
            if slot
                .as_ref()
                .is_some_and(|cached| cached.paint_key.uses_semantic_color())
            {
                *slot = None;
            }
        }
    }
}

pub struct RenderOutput {
    pub image: Retained<NSImage>,
    pub layout: LayoutDiagnostics,
    pub font: FontResolution,
    pub identifier_resolved: [bool; 2],
}

pub struct PreviewImages {
    pub light: Retained<NSImage>,
    pub dark: Retained<NSImage>,
}

fn label_prefix(runs: &[IndicatorRun]) -> Option<String> {
    match runs {
        [label, value] if value.text.ends_with('%') => Some(label.text.clone()),
        [combined] => combined
            .text
            .rsplit_once(char::is_whitespace)
            .filter(|(_, value)| value.ends_with('%'))
            .map(|(label, _)| format!("{label} ")),
        _ => None,
    }
}

fn metric_prefix(
    runs: &[IndicatorRun],
    identifier: Option<&MetricIdentifierVisual>,
) -> Option<String> {
    identifier
        .map(|identifier| match identifier {
            MetricIdentifierVisual::SystemSymbol { fallback_text, .. }
            | MetricIdentifierVisual::Png { fallback_text, .. } => fallback_text.clone(),
        })
        .or_else(|| label_prefix(runs))
}

fn metric_spacing_level(runs: &[IndicatorRun]) -> u8 {
    match runs {
        [label, _] => label.trailing_spacing_level,
        _ => 0,
    }
}

type TextAttributes = Retained<NSDictionary<NSString, AnyObject>>;

#[derive(Clone)]
struct ResolvedTypography {
    generation: u64,
    preferences: TypographyPreferences,
    font: FontResolution,
    measurer: FontTextMeasurer,
}

#[derive(Default)]
struct ResolvedTypographyCache {
    active: Option<ResolvedTypography>,
}

impl ResolvedTypographyCache {
    fn matches(&self, generation: u64, preferences: &TypographyPreferences) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.generation == generation && &active.preferences == preferences
        })
    }

    fn replace(&mut self, resolved: ResolvedTypography) {
        self.active = Some(resolved);
    }

    fn get(&self) -> ResolvedTypography {
        self.active
            .as_ref()
            .expect("typography is resolved before rendering")
            .clone()
    }

    fn clear(&mut self) {
        self.active = None;
    }
}

const ATTRIBUTE_CACHE_CAPACITY: usize = 16;

struct AttributeEntry {
    key: PaintIdentity,
    attributes: TextAttributes,
}

#[derive(Default)]
struct AttributeCache {
    entries: Vec<AttributeEntry>,
    total_created: usize,
}

impl AttributeCache {
    fn resolve(
        &mut self,
        key: PaintIdentity,
        create: impl FnOnce() -> TextAttributes,
    ) -> TextAttributes {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let entry = self.entries.remove(index);
            let attributes = entry.attributes.clone();
            self.entries.push(entry);
            return attributes;
        }
        let attributes = create();
        self.total_created += 1;
        if self.entries.len() == ATTRIBUTE_CACHE_CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push(AttributeEntry {
            key,
            attributes: attributes.clone(),
        });
        attributes
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn clear_semantic(&mut self) {
        self.entries
            .retain(|entry| matches!(entry.key, PaintIdentity::Srgb(_)));
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct ConstructionStats {
    font_resolutions: usize,
    measurers: usize,
    attribute_sets: usize,
    images: usize,
}

pub struct Renderer {
    font_catalog: FontCatalog,
    font_generation: u64,
    typography: ResolvedTypographyCache,
    attributes: AttributeCache,
    cache: SurfaceCache<Retained<NSImage>>,
    identifier_images: IdentifierImageCache<Retained<NSImage>>,
    icon_asset_store: IconAssetStore,
    presentation: RuntimePresentation,
    default_width: f64,
    font_resolutions: usize,
    measurers: usize,
    images: usize,
}

impl Renderer {
    pub fn new(icon_asset_store: IconAssetStore, presentation: RuntimePresentation) -> Self {
        let marker = MainThreadMarker::new().expect("Renderer must be created on the main thread");
        Self::with_main_thread_marker(marker, icon_asset_store, presentation)
    }

    pub fn with_main_thread_marker(
        marker: MainThreadMarker,
        icon_asset_store: IconAssetStore,
        presentation: RuntimePresentation,
    ) -> Self {
        let font_catalog = FontCatalog::new(marker);
        let default_typography =
            statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let default_font = font_catalog.resolve(&default_typography);
        let default_measurer = FontTextMeasurer::new(&default_font.font);
        let default_width =
            measure_stable_layout(&default_measurer, true, f64::INFINITY).base_width();
        let mut typography = ResolvedTypographyCache::default();
        typography.replace(ResolvedTypography {
            generation: 0,
            preferences: default_typography,
            font: default_font,
            measurer: default_measurer,
        });
        Self {
            font_catalog,
            font_generation: 0,
            typography,
            attributes: AttributeCache::default(),
            cache: SurfaceCache::default(),
            identifier_images: IdentifierImageCache::default(),
            icon_asset_store,
            presentation,
            default_width,
            font_resolutions: 1,
            measurers: 1,
            images: 0,
        }
    }

    pub fn render(
        &mut self,
        slot: RenderSlot,
        scene: &IndicatorScene,
        typography: &TypographyPreferences,
        appearance: &NSAppearance,
    ) -> RenderOutput {
        let resolved = self.resolve_typography(typography);
        let font = resolved.font;
        let measurer = resolved.measurer;
        let cpu_prefix = metric_prefix(&scene.top, scene.top_identifier.as_ref());
        let ram_prefix = metric_prefix(&scene.bottom, scene.bottom_identifier.as_ref());
        let cpu_spacing_level = metric_spacing_level(&scene.top);
        let ram_spacing_level = metric_spacing_level(&scene.bottom);
        let marker_plan = if slot == RenderSlot::Status {
            status_marker_plan(&self.presentation, |text| measurer.width(text))
        } else {
            StatusMarkerPlan {
                text: None,
                extra_width: 0.0,
            }
        };
        let layout_key = LayoutKey {
            resolved_family: font.resolved_family.clone(),
            size: typography.size.points(),
            weight: typography.weight,
            cpu_prefix,
            ram_prefix,
            cpu_spacing_level,
            ram_spacing_level,
            dev_marker: marker_plan.text.clone(),
        };
        let layout = self.cache.resolve_layout(slot, &layout_key, || {
            measure_stable_layout_with_prefixes_and_spacing(
                &measurer,
                layout_key.cpu_prefix.as_deref(),
                layout_key.ram_prefix.as_deref(),
                layout_key.cpu_spacing_level,
                layout_key.ram_spacing_level,
                self.default_width,
            )
        });
        let appearance_name = appearance.name().to_string();
        let paint_key = paint_key(scene, layout_key.clone(), appearance_name.clone());
        let top_identifier_image = self.resolve_identifier_image(
            scene.top_identifier.as_ref(),
            typography,
            &appearance_name,
        );
        let bottom_identifier_image = self.resolve_identifier_image(
            scene.bottom_identifier.as_ref(),
            typography,
            &appearance_name,
        );
        let identifier_resolved = [
            scene.top_identifier.is_none() || top_identifier_image.is_some(),
            scene.bottom_identifier.is_none() || bottom_identifier_image.is_some(),
        ];
        if let Some(image) = self.cache.reused_image(slot, &paint_key) {
            return RenderOutput {
                image,
                layout: layout.diagnostics,
                font,
                identifier_resolved,
            };
        }
        let image = draw_image(
            scene,
            IdentifierImages {
                top: top_identifier_image.as_deref(),
                bottom: bottom_identifier_image.as_deref(),
            },
            &font.font,
            &measurer,
            layout,
            &mut self.attributes,
            ImageRenderEnvironment {
                appearance,
                marker: marker_plan,
            },
        );
        self.images += 1;

        self.cache
            .replace(slot, layout_key, layout, paint_key, image.clone());

        RenderOutput {
            image,
            layout: layout.diagnostics,
            font,
            identifier_resolved,
        }
    }

    pub(crate) fn apply_status(
        &mut self,
        target: &impl StatusRenderTarget,
        scene: &IndicatorScene,
        typography: &TypographyPreferences,
        appearance: &NSAppearance,
    ) -> LayoutDiagnostics {
        let output = self.render(RenderSlot::Status, scene, typography, appearance);
        target.set_status_image(&output.image);
        apply_status_metadata(target, scene, &self.presentation);
        output.layout
    }

    pub fn refresh_fonts(&mut self) {
        self.font_catalog.refresh();
        self.font_generation = self.font_generation.wrapping_add(1);
        self.typography.clear();
        self.attributes.clear();
        self.cache.clear();
    }

    pub fn invalidate_semantic_colors(&mut self) {
        self.attributes.clear_semantic();
        self.cache.clear_semantic_paint();
        self.identifier_images.clear_semantic();
    }

    fn resolve_typography(&mut self, preferences: &TypographyPreferences) -> ResolvedTypography {
        if !self.typography.matches(self.font_generation, preferences) {
            let font = self.font_catalog.resolve(preferences);
            let measurer = FontTextMeasurer::new(&font.font);
            self.font_resolutions += 1;
            self.measurers += 1;
            self.typography.replace(ResolvedTypography {
                generation: self.font_generation,
                preferences: preferences.clone(),
                font,
                measurer,
            });
            self.attributes.clear();
            self.cache.clear();
        }
        self.typography.get()
    }

    fn resolve_identifier_image(
        &mut self,
        identifier: Option<&MetricIdentifierVisual>,
        typography: &TypographyPreferences,
        appearance: &str,
    ) -> Option<Retained<NSImage>> {
        let identifier = identifier?;
        let key = identifier_image_key(identifier, typography, appearance);
        let icon_asset_store = &self.icon_asset_store;
        self.identifier_images.resolve(key, || {
            let image = match identifier {
                MetricIdentifierVisual::SystemSymbol { name, color, .. } => {
                    create_system_symbol_image(
                        name,
                        &resolve_color(*color),
                        f64::from(typography.size.points()),
                        typography.weight,
                    )
                }
                MetricIdentifierVisual::Png { metric, .. } => {
                    let path = icon_asset_store.path_for(*metric);
                    let path = path.to_str()?;
                    NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path))
                }
            }?;
            let image_size = image.size();
            (image_size.width > 0.0 && image_size.height > 0.0).then_some(image)
        })
    }

    #[cfg(test)]
    fn construction_stats(&self) -> ConstructionStats {
        ConstructionStats {
            font_resolutions: self.font_resolutions,
            measurers: self.measurers,
            attribute_sets: self.attributes.total_created,
            images: self.images,
        }
    }

    #[cfg(test)]
    fn attribute_cache_len(&self) -> usize {
        self.attributes.entries.len()
    }
}

#[derive(Clone)]
struct FontTextMeasurer {
    font: Retained<NSFont>,
    attributes: TextAttributes,
}

impl FontTextMeasurer {
    fn new(font: &NSFont) -> Self {
        Self {
            font: font.retain(),
            attributes: NSDictionary::from_retained_objects(
                &[unsafe { NSFontAttributeName }],
                &[Retained::into_super(Retained::into_super(font.retain()))],
            ),
        }
    }
}

impl TextMeasurer for FontTextMeasurer {
    fn width(&self, text: &str) -> f64 {
        unsafe {
            objc2_foundation::NSAttributedString::new_with_attributes(
                &NSString::from_str(text),
                &self.attributes,
            )
        }
        .size()
        .width
    }

    fn content_height(&self) -> f64 {
        2.0 * self.font.capHeight() + LINE_GAP
    }
}

struct IdentifierImages<'a> {
    top: Option<&'a NSImage>,
    bottom: Option<&'a NSImage>,
}

struct ImageRenderEnvironment<'a> {
    appearance: &'a NSAppearance,
    marker: StatusMarkerPlan,
}

fn draw_image(
    scene: &IndicatorScene,
    identifier_images: IdentifierImages<'_>,
    font: &NSFont,
    measurer: &FontTextMeasurer,
    layout: StableLayout,
    attributes: &mut AttributeCache,
    environment: ImageRenderEnvironment<'_>,
) -> Retained<NSImage> {
    let appearance = environment.appearance;
    let marker = environment.marker;
    let rendered = RefCell::new(None);
    let attributes = RefCell::new(attributes);
    let appearance_name = appearance.name().to_string();
    let draw = StackBlock::new(|| {
        let badge = scene.disk_badge.as_ref().map(|run| run.text.as_str());
        let content_width = layout.width_for_badge(badge);
        let width = (content_width + marker.extra_width).ceil();
        let image = NSImage::initWithSize(
            NSImage::alloc(),
            NSSize {
                width,
                height: HEIGHT,
            },
        );
        let cap_height = font.capHeight();
        let descent = -font.descender();
        let margin = (HEIGHT - 2.0 * cap_height - LINE_GAP) / 2.0;
        let mut attributes = attributes.borrow_mut();
        let mut text = DrawTextContext {
            font,
            measurer,
            layout,
            attributes: &mut attributes,
            appearance_name: &appearance_name,
        };

        #[allow(deprecated)]
        {
            image.lockFocus();
            draw_metric_line(
                &scene.bottom,
                scene.bottom_identifier.as_ref(),
                identifier_images.bottom,
                margin - descent,
                layout.ram_width,
                &mut text,
            );
            draw_metric_line(
                &scene.top,
                scene.top_identifier.as_ref(),
                identifier_images.top,
                margin + cap_height + LINE_GAP - descent,
                layout.cpu_width,
                &mut text,
            );
            if let Some(badge) = &scene.disk_badge {
                attributed_scene_run(text.font, badge, text.attributes, text.appearance_name)
                    .drawAtPoint(NSPoint {
                        x: layout.base_width(),
                        y: margin + cap_height + LINE_GAP - descent,
                    });
            }
            if let Some(marker) = &marker.text {
                attributed_scene_run(
                    text.font,
                    &IndicatorRun {
                        text: marker.clone(),
                        color: SegmentColor::Semantic(SemanticColor::Neutral),
                        trailing_spacing_level: 0,
                    },
                    text.attributes,
                    text.appearance_name,
                )
                .drawAtPoint(NSPoint {
                    x: content_width,
                    y: (HEIGHT - cap_height) / 2.0 - descent,
                });
            }
            image.unlockFocus();
        }
        rendered.replace(Some(image));
    });
    unsafe {
        let _: () = objc2::msg_send![
            appearance,
            performAsCurrentDrawingAppearance: &*draw
        ];
    }
    rendered
        .into_inner()
        .expect("drawing appearance executes its block synchronously")
}

struct DrawTextContext<'a> {
    font: &'a NSFont,
    measurer: &'a FontTextMeasurer,
    layout: StableLayout,
    attributes: &'a mut AttributeCache,
    appearance_name: &'a str,
}

fn draw_metric_line(
    runs: &[IndicatorRun],
    identifier: Option<&MetricIdentifierVisual>,
    identifier_image: Option<&NSImage>,
    y: f64,
    line_width: f64,
    context: &mut DrawTextContext<'_>,
) {
    if let (Some(identifier), [value]) = (identifier, runs) {
        let (fallback_text, fallback_color) = match identifier {
            MetricIdentifierVisual::SystemSymbol {
                color,
                fallback_text,
                ..
            }
            | MetricIdentifierVisual::Png {
                fallback_color: color,
                fallback_text,
                ..
            } => (fallback_text, *color),
        };
        let prefix_width = context.measurer.width(fallback_text);
        if let Some(image) = identifier_image {
            draw_metric_identifier(
                image,
                y,
                prefix_width,
                matches!(identifier, MetricIdentifierVisual::SystemSymbol { .. }),
                context,
            );
        } else {
            attributed_scene_run(
                context.font,
                &IndicatorRun {
                    text: fallback_text.clone(),
                    color: fallback_color,
                    trailing_spacing_level: 0,
                },
                context.attributes,
                context.appearance_name,
            )
            .drawAtPoint(NSPoint { x: 0.0, y });
        }
        attributed_scene_run(
            context.font,
            value,
            context.attributes,
            context.appearance_name,
        )
        .drawAtPoint(NSPoint { x: prefix_width, y });
        return;
    }
    match runs {
        [value] => attributed_scene_run(
            context.font,
            value,
            context.attributes,
            context.appearance_name,
        )
        .drawAtPoint(NSPoint {
            x: context
                .layout
                .value_origin(context.measurer, line_width, &value.text),
            y,
        }),
        [label, value] => {
            attributed_scene_run(
                context.font,
                label,
                context.attributes,
                context.appearance_name,
            )
            .drawAtPoint(NSPoint { x: 0.0, y });
            attributed_scene_run(
                context.font,
                value,
                context.attributes,
                context.appearance_name,
            )
            .drawAtPoint(NSPoint {
                x: label_value_origin(context.measurer, &label.text)
                    + trailing_spacing_width(
                        context.measurer,
                        &label.text,
                        label.trailing_spacing_level,
                    ),
                y,
            });
        }
        _ => attributed_scene_line(
            context.font,
            runs.iter(),
            context.attributes,
            context.appearance_name,
        )
        .drawAtPoint(NSPoint { x: 0.0, y }),
    }
}

fn draw_metric_identifier(
    image: &NSImage,
    y: f64,
    prefix_width: f64,
    is_system_symbol: bool,
    context: &DrawTextContext<'_>,
) {
    let spacing = context.measurer.width(" ");
    let available_height = context.font.capHeight().max(1.0);
    let source = image.size();
    let rect = identifier_draw_rect(
        source.width,
        source.height,
        prefix_width,
        available_height,
        spacing,
        y,
        is_system_symbol,
    );
    image.drawInRect(NSRect::new(
        NSPoint::new(rect.x, rect.y),
        NSSize::new(rect.width, rect.height),
    ));
}

struct IdentifierDrawRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn identifier_draw_rect(
    source_width: f64,
    source_height: f64,
    prefix_width: f64,
    line_height: f64,
    spacing: f64,
    y: f64,
    is_system_symbol: bool,
) -> IdentifierDrawRect {
    let (drawing_width, horizontal_width) = if is_system_symbol {
        (line_height.max(1.0), prefix_width)
    } else {
        let width = (prefix_width - spacing).max(1.0);
        (width, width)
    };
    let drawing_height = line_height.max(1.0);
    let scale =
        (drawing_width / source_width.max(1.0)).min(drawing_height / source_height.max(1.0));
    let width = source_width * scale;
    let height = source_height * scale;
    IdentifierDrawRect {
        x: (horizontal_width - width) / 2.0,
        y: y + if is_system_symbol {
            (drawing_height - height) / 2.0
        } else {
            0.0
        },
        width,
        height,
    }
}

fn create_system_symbol_image(
    name: &statlet::indicator_preferences::SystemSymbolName,
    color: &NSColor,
    point_size: f64,
    weight: FontWeight,
) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name.as_str()),
        None,
    )?;
    let weight = unsafe {
        match weight {
            FontWeight::Regular => NSFontWeightRegular,
            FontWeight::Medium => NSFontWeightMedium,
            FontWeight::Bold => NSFontWeightBold,
        }
    };
    let size = NSImageSymbolConfiguration::configurationWithPointSize_weight(point_size, weight);
    let color = NSImageSymbolConfiguration::configurationWithHierarchicalColor(color);
    let configuration = size.configurationByApplyingConfiguration(&color);
    image.imageWithSymbolConfiguration(&configuration)
}

fn label_value_origin(measurer: &impl TextMeasurer, label: &str) -> f64 {
    measurer.width(label)
}

fn attributed_scene_run(
    font: &NSFont,
    run: &IndicatorRun,
    cache: &mut AttributeCache,
    appearance_name: &str,
) -> Retained<objc2_foundation::NSAttributedString> {
    let key = paint_identity(run.color, appearance_name);
    let attributes = cache.resolve(key, || {
        let color = resolve_color(run.color);
        NSDictionary::from_retained_objects(
            &[unsafe { NSFontAttributeName }, unsafe {
                NSForegroundColorAttributeName
            }],
            &[
                Retained::into_super(Retained::into_super(font.retain())),
                Retained::into_super(Retained::into_super(color)),
            ],
        )
    });
    unsafe {
        objc2_foundation::NSAttributedString::new_with_attributes(
            &NSString::from_str(&run.text),
            &attributes,
        )
    }
}

fn attributed_scene_line<'a>(
    font: &NSFont,
    runs: impl Iterator<Item = &'a IndicatorRun>,
    attributes: &mut AttributeCache,
    appearance_name: &str,
) -> Retained<NSMutableAttributedString> {
    let line = NSMutableAttributedString::new();
    for run in runs {
        line.appendAttributedString(&attributed_scene_run(
            font,
            run,
            attributes,
            appearance_name,
        ));
    }
    line
}

fn resolve_color(color: SegmentColor) -> Retained<NSColor> {
    match color_plan(color) {
        ColorPlan::Semantic(color) => semantic_color(color),
        ColorPlan::Srgb(components) => {
            let [red, green, blue] = components.map(|component| f64::from(component) / 255.0);
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0);
            color
                .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
                .unwrap_or(color)
        }
    }
}

pub fn resolved_scene_srgb_colors(
    scene: &IndicatorScene,
    appearance: &NSAppearance,
) -> Vec<[f64; 3]> {
    let resolved = RefCell::new(None);
    let resolve = StackBlock::new(|| {
        let colors = scene
            .top_identifier
            .as_ref()
            .and_then(identifier_color)
            .into_iter()
            .chain(scene.top.iter().map(|run| run.color))
            .chain(scene.bottom_identifier.as_ref().and_then(identifier_color))
            .chain(scene.bottom.iter().map(|run| run.color))
            .chain(scene.disk_badge.iter().map(|run| run.color))
            .map(|color| resolved_srgb_components(&resolve_color(color)))
            .collect();
        resolved.replace(Some(colors));
    });
    unsafe {
        let _: () = objc2::msg_send![
            appearance,
            performAsCurrentDrawingAppearance: &*resolve
        ];
    }
    resolved
        .into_inner()
        .expect("drawing appearance resolves colors synchronously")
}

fn identifier_color(identifier: &MetricIdentifierVisual) -> Option<SegmentColor> {
    match identifier {
        MetricIdentifierVisual::SystemSymbol { color, .. } => Some(*color),
        MetricIdentifierVisual::Png { .. } => None,
    }
}

fn resolved_srgb_components(color: &NSColor) -> [f64; 3] {
    let color = color
        .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        .expect("indicator colors convert to sRGB");
    [
        color.redComponent(),
        color.greenComponent(),
        color.blueComponent(),
    ]
}

fn semantic_color(color: SemanticColor) -> Retained<NSColor> {
    match color {
        SemanticColor::Neutral => NSColor::labelColor(),
        SemanticColor::Good => NSColor::systemGreenColor(),
        SemanticColor::Warning => NSColor::systemOrangeColor(),
        SemanticColor::Critical => NSColor::systemRedColor(),
        SemanticColor::DiskWarning => NSColor::systemYellowColor(),
        SemanticColor::DiskError => NSColor::systemRedColor(),
    }
}

pub fn status_button(marker: MainThreadMarker) -> Option<Retained<NSStatusBarButton>> {
    let app = NSApplication::sharedApplication(marker);
    for window in app.windows() {
        if window.class().name().to_bytes() == b"NSStatusBarWindow" {
            if let Some(view) = window.contentView() {
                if let Some(button) = find_button(&view) {
                    return Some(button);
                }
            }
        }
    }
    None
}

fn find_button(view: &NSView) -> Option<Retained<NSStatusBarButton>> {
    if let Ok(button) = view.retain().downcast::<NSStatusBarButton>() {
        return Some(button);
    }
    for subview in view.subviews() {
        if let Some(button) = find_button(&subview) {
            return Some(button);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use tempfile::tempdir;

    fn production_renderer(marker: MainThreadMarker) -> Renderer {
        Renderer::with_main_thread_marker(
            marker,
            IconAssetStore::new(std::path::PathBuf::from(
                "/nonexistent/statlet-renderer-test-icons",
            )),
            RuntimePresentation::default(),
        )
    }

    fn development_presentation() -> RuntimePresentation {
        statlet::runtime_profile::RuntimeProfile::resolve(
            statlet::runtime_profile::BundleProfileMetadata {
                bundle_identifier: Some(
                    "io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into(),
                ),
                runtime_profile: Some("development".into()),
                dev_instance_id: Some("task-a-0123456789ab".into()),
                dev_display_name: Some("Task A".into()),
                dev_short_marker: Some("0123".into()),
            },
        )
        .unwrap()
        .presentation()
    }

    use statlet::indicator::{
        has_low_text_contrast, measure_stable_layout, IndicatorRun, IndicatorScene,
        MetricIdentifierVisual, PreviewBackground, SegmentColor, SemanticColor, TextMeasurer,
    };
    use statlet::indicator_preferences::{
        FontWeight, MetricKind, PngIconMetadata, SrgbColor, SystemSymbolName,
    };

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestEntry(u8);

    #[test]
    fn identifier_image_cache_keeps_failures_until_the_resolution_key_changes() {
        let calls = Cell::new(0);
        let mut cache = IdentifierImageCache::default();
        let key = IdentifierImageKey {
            identity: IdentifierImageIdentity::Png {
                metric: MetricKind::Cpu,
                metadata: PngIconMetadata::new("cpu.png", 12, 12, 400).unwrap(),
            },
            size: 12,
            weight: FontWeight::Medium,
        };

        assert_eq!(
            cache.resolve(key.clone(), || {
                calls.set(calls.get() + 1);
                None::<TestEntry>
            }),
            None
        );
        assert_eq!(
            cache.resolve(key.clone(), || {
                calls.set(calls.get() + 1);
                Some(TestEntry(1))
            }),
            None
        );
        assert_eq!(calls.get(), 1);

        cache.clear_semantic();
        assert_eq!(
            cache.resolve(key, || {
                calls.set(calls.get() + 1);
                Some(TestEntry(1))
            }),
            None
        );
        assert_eq!(calls.get(), 1);

        let changed = IdentifierImageKey {
            identity: IdentifierImageIdentity::Png {
                metric: MetricKind::Cpu,
                metadata: PngIconMetadata::new("cpu.png", 12, 12, 401).unwrap(),
            },
            size: 12,
            weight: FontWeight::Medium,
        };
        assert_eq!(
            cache.resolve(changed, || {
                calls.set(calls.get() + 1);
                Some(TestEntry(2))
            }),
            Some(TestEntry(2))
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn missing_png_is_reported_as_unresolved_for_preview_accessibility() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let directory = tempdir().unwrap();
        let mut renderer = Renderer::with_main_thread_marker(
            marker,
            IconAssetStore::new(directory.path().to_path_buf()),
            RuntimePresentation::default(),
        );
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let mut scene = scene_with_lines(&["42%"], &["R ", "68%"]);
        scene.top_identifier = Some(MetricIdentifierVisual::Png {
            metric: MetricKind::Cpu,
            metadata: PngIconMetadata::new("missing.png", 12, 12, 400).unwrap(),
            fallback_color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".into(),
        });
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();

        let output = renderer.render(RenderSlot::PreviewLight, &scene, &typography, &aqua);

        assert_eq!(output.identifier_resolved, [false, true]);
    }

    #[test]
    fn failed_png_resolution_is_not_retried_until_the_asset_preference_changes() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let directory = tempdir().unwrap();
        let mut renderer = Renderer::with_main_thread_marker(
            marker,
            IconAssetStore::new(directory.path().to_path_buf()),
            RuntimePresentation::default(),
        );
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let mut scene = scene_with_lines(&["42%"], &["R ", "68%"]);
        scene.top_identifier = Some(MetricIdentifierVisual::Png {
            metric: MetricKind::Cpu,
            metadata: PngIconMetadata::new("missing.png", 12, 12, 400).unwrap(),
            fallback_color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".into(),
        });
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();
        let dark =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua })
                .unwrap();

        let first = renderer.render(RenderSlot::Status, &scene, &typography, &aqua);
        assert_eq!(first.identifier_resolved, [false, true]);

        fs::create_dir_all(directory.path()).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(12, 12, Rgba([0, 0, 0, 0xFF])))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        fs::write(directory.path().join("cpu.png"), &bytes).unwrap();

        renderer.invalidate_semantic_colors();
        let redraw = renderer.render(RenderSlot::Status, &scene, &typography, &dark);
        assert_eq!(redraw.identifier_resolved, [false, true]);

        if let Some(MetricIdentifierVisual::Png { metadata, .. }) = &mut scene.top_identifier {
            *metadata = PngIconMetadata::new("cpu.png", 12, 12, bytes.len() as u64).unwrap();
        }
        let changed = renderer.render(RenderSlot::Status, &scene, &typography, &dark);
        assert_eq!(changed.identifier_resolved, [true, true]);
    }

    #[test]
    fn rerender_replaces_one_entry_per_surface_instead_of_accumulating() {
        let scene = scene_with_lines(&["C 42%"], &["R 68%"]);
        let mut cache = SurfaceCache::default();

        cache_test_surface(&mut cache, RenderSlot::Status, &scene, TestEntry(1));
        cache_test_surface(&mut cache, RenderSlot::PreviewLight, &scene, TestEntry(2));
        cache_test_surface(&mut cache, RenderSlot::PreviewDark, &scene, TestEntry(3));
        cache_test_surface(&mut cache, RenderSlot::Status, &scene, TestEntry(4));

        let key = paint_key(
            &scene,
            layout_key("Menlo", true),
            "NSAppearanceNameAqua".to_owned(),
        );
        assert_eq!(cache.slots.entries.iter().flatten().count(), 3);
        assert_eq!(
            cache.reused_image(RenderSlot::Status, &key),
            Some(TestEntry(4))
        );
    }

    struct CountingMeasurer {
        calls: Cell<usize>,
    }

    impl CountingMeasurer {
        fn new() -> Self {
            Self {
                calls: Cell::new(0),
            }
        }
    }

    impl TextMeasurer for CountingMeasurer {
        fn width(&self, text: &str) -> f64 {
            self.calls.set(self.calls.get() + 1);
            text.len() as f64
        }

        fn content_height(&self) -> f64 {
            18.0
        }
    }

    #[test]
    fn label_value_origin_uses_only_the_literal_label_prefix() {
        let measurer = CountingMeasurer::new();

        for label in ["C", "C ", "C  ", "C   ", "CPU    "] {
            for value in ["0%", "9%", "10%", "99%", "100%"] {
                assert_eq!(
                    label_value_origin(&measurer, label),
                    measurer.width(label),
                    "{label:?} before {value}"
                );
            }
        }
    }

    #[test]
    fn system_symbol_draw_rect_uses_a_line_height_square_box() {
        let rect = identifier_draw_rect(12.0, 12.0, 12.0, 10.0, 4.0, 2.0, true);

        assert_eq!(rect.x, 1.0);
        assert_eq!(rect.y, 2.0);
        assert_eq!(rect.width, 10.0);
        assert_eq!(rect.height, 10.0);
    }

    #[test]
    fn png_draw_rect_keeps_the_legacy_prefix_width_limit() {
        let rect = identifier_draw_rect(12.0, 12.0, 11.0, 10.0, 4.0, 2.0, false);

        assert_eq!(rect.x, 0.0);
        assert_eq!(rect.y, 2.0);
        assert_eq!(rect.width, 7.0);
        assert_eq!(rect.height, 7.0);
    }

    fn layout_key(family: &str, labels_visible: bool) -> LayoutKey {
        LayoutKey {
            resolved_family: family.to_owned(),
            size: 12,
            weight: FontWeight::Medium,
            cpu_prefix: labels_visible.then(|| "C ".to_owned()),
            ram_prefix: labels_visible.then(|| "R ".to_owned()),
            cpu_spacing_level: 0,
            ram_spacing_level: 0,
            dev_marker: None,
        }
    }

    #[test]
    fn paint_only_change_reuses_layout_but_typography_and_labels_invalidate_it() {
        let measurer = CountingMeasurer::new();
        let scene = scene_with_lines(&["C ", "42%"], &["R ", "68%"]);
        let mut cache = SurfaceCache::default();
        let key = layout_key("Menlo", true);

        let initial = cache.resolve_layout(RenderSlot::Status, &key, || {
            measure_stable_layout(&measurer, true, 40.0)
        });
        cache.replace(
            RenderSlot::Status,
            key.clone(),
            initial,
            paint_key(&scene, key.clone(), "NSAppearanceNameAqua".to_owned()),
            TestEntry(1),
        );
        let initial_calls = measurer.calls.get();
        cache.resolve_layout(RenderSlot::Status, &key, || {
            measure_stable_layout(&measurer, true, 40.0)
        });
        assert_eq!(measurer.calls.get(), initial_calls);

        let hidden_key = layout_key("Menlo", false);
        let hidden = cache.resolve_layout(RenderSlot::Status, &hidden_key, || {
            measure_stable_layout(&measurer, false, 40.0)
        });
        assert!(measurer.calls.get() > initial_calls);
        cache.replace(
            RenderSlot::Status,
            hidden_key.clone(),
            hidden,
            paint_key(&scene, hidden_key, "NSAppearanceNameAqua".to_owned()),
            TestEntry(2),
        );
        let after_labels = measurer.calls.get();

        cache.resolve_layout(
            RenderSlot::Status,
            &layout_key("Avenir Next", false),
            || measure_stable_layout(&measurer, false, 40.0),
        );
        assert!(measurer.calls.get() > after_labels);

        let after_typography = measurer.calls.get();
        let mut decimal_spacing_key = layout_key("Menlo", false);
        decimal_spacing_key.cpu_spacing_level = 5;
        cache.resolve_layout(RenderSlot::Status, &decimal_spacing_key, || {
            measure_stable_layout(&measurer, false, 40.0)
        });
        assert!(
            measurer.calls.get() > after_typography,
            "a decimal spacing level must invalidate the layout cache"
        );
    }

    #[test]
    fn production_color_plan_keeps_fixed_srgb_and_semantic_appearance_identity() {
        let fixed = SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap());
        let semantic = SegmentColor::Semantic(SemanticColor::Warning);

        assert_eq!(color_plan(fixed), ColorPlan::Srgb([0xAF, 0x52, 0xDE]));
        assert_eq!(
            color_plan(semantic),
            ColorPlan::Semantic(SemanticColor::Warning)
        );
        assert_eq!(
            paint_identity(fixed, "NSAppearanceNameAqua"),
            paint_identity(fixed, "NSAppearanceNameDarkAqua")
        );
        assert_ne!(
            paint_identity(semantic, "NSAppearanceNameAqua"),
            paint_identity(semantic, "NSAppearanceNameDarkAqua")
        );
        assert_eq!(
            paint_identity(fixed, "NSAppearanceNameAqua"),
            PaintIdentity::Srgb([0xAF, 0x52, 0xDE])
        );
    }

    fn semantic_warning_scene() -> IndicatorScene {
        IndicatorScene {
            top: vec![IndicatorRun {
                text: "C 42%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Warning),
                trailing_spacing_level: 0,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Warning),
                trailing_spacing_level: 0,
            }],
            top_identifier: None,
            bottom_identifier: None,
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%".to_owned(),
        }
    }

    #[test]
    fn aqua_semantic_runs_resolve_to_srgb_before_contrast_diagnostics() {
        let appearance =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();

        let colors = resolved_scene_srgb_colors(&semantic_warning_scene(), &appearance);

        assert_eq!(colors.len(), 2);
        assert!(has_low_text_contrast(&colors, PreviewBackground::Light));
    }

    #[test]
    fn dark_aqua_semantic_runs_resolve_to_srgb_before_contrast_diagnostics() {
        let appearance =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua })
                .unwrap();
        let colors = resolved_scene_srgb_colors(&semantic_warning_scene(), &appearance);

        assert_eq!(colors.len(), 2);
        assert!(!has_low_text_contrast(&colors, PreviewBackground::Dark));
    }

    #[test]
    fn symbol_color_precedes_its_value_in_resolved_preview_colors() {
        let mut scene = scene_with_lines(&["42%"], &["68%"]);
        scene.top_identifier = Some(MetricIdentifierVisual::SystemSymbol {
            name: SystemSymbolName::new("cpu").unwrap(),
            color: SegmentColor::Srgb(SrgbColor::parse_hex("#112233").unwrap()),
            fallback_text: "C ".into(),
        });
        let appearance =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();

        let colors = resolved_scene_srgb_colors(&scene, &appearance);

        assert_eq!(colors.len(), 3);
        assert_eq!(
            colors[0],
            [0x11, 0x22, 0x33].map(|value| f64::from(value) / 255.0)
        );
    }

    #[test]
    fn accessible_status_metadata_is_applied_to_the_target() {
        let scene = IndicatorScene {
            top: vec![IndicatorRun {
                text: "42%".to_owned(),
                color: SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap()),
                trailing_spacing_level: 0,
            }],
            bottom: vec![IndicatorRun {
                text: "68%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Good),
                trailing_spacing_level: 0,
            }],
            top_identifier: None,
            bottom_identifier: None,
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%, pressão de memória normal".to_owned(),
        };

        let target = FakeStatusTarget::default();

        apply_status_metadata(&target, &scene, &RuntimePresentation::default());

        assert_eq!(
            target.accessibility_label.borrow().as_deref(),
            Some(scene.accessibility_label.as_str())
        );
        assert_eq!(
            target.tooltip.borrow().as_deref(),
            Some(scene.accessibility_label.as_str())
        );
    }

    #[test]
    fn development_status_metadata_identifies_the_bundle_in_tooltip_and_accessibility() {
        let scene = scene_with_lines(&["42%"], &["68%"]);
        let presentation = development_presentation();
        let target = FakeStatusTarget::default();

        apply_status_metadata(&target, &scene, &presentation);

        let expected = format!(
            "Statlet Dev — Task A (task-a-0123456789ab): {}",
            scene.accessibility_label
        );
        assert_eq!(
            target.accessibility_label.borrow().as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(target.tooltip.borrow().as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn development_marker_has_its_own_status_image_column_beside_the_disk_badge() {
        let production =
            status_marker_plan(&RuntimePresentation::default(), |text| text.len() as f64);
        let development = status_marker_plan(&development_presentation(), |text| text.len() as f64);

        assert_eq!(production.text, None);
        assert_eq!(production.extra_width, 0.0);
        assert_eq!(development.text.as_deref(), Some(" D:0123"));
        assert_eq!(development.extra_width, 7.0);
    }

    #[test]
    fn development_marker_participates_in_semantic_paint_and_surface_cache_identity() {
        let scene = scene_with_lines(&["C 42%"], &["R 68%"]);
        let mut marked_layout = layout_key("Menlo", true);
        marked_layout.dev_marker = Some(" D:0123".to_owned());
        let paint_keys = [
            "NSAppearanceNameAqua",
            "NSAppearanceNameDarkAqua",
            "NSAppearanceNameAccessibilityHighContrastAqua",
            "NSAppearanceNameAccessibilityHighContrastDarkAqua",
        ]
        .map(|appearance| paint_key(&scene, marked_layout.clone(), appearance.to_owned()));
        let aqua = &paint_keys[0];

        assert_eq!(
            aqua.dev_marker.as_ref().map(|marker| marker.text.as_str()),
            Some(" D:0123")
        );
        assert!(paint_keys.iter().all(PaintKey::uses_semantic_color));
        for (index, key) in paint_keys.iter().enumerate() {
            assert!(paint_keys.iter().skip(index + 1).all(|other| other != key));
        }

        let layout = measure_stable_layout(&CountingMeasurer::new(), true, 40.0);
        let mut cache = SurfaceCache::default();
        cache.replace(
            RenderSlot::Status,
            marked_layout,
            layout,
            aqua.clone(),
            TestEntry(7),
        );
        for requested in &paint_keys[1..] {
            assert_eq!(cache.reused_image(RenderSlot::Status, requested), None);
        }
    }

    #[test]
    fn development_marker_renders_in_standard_and_high_contrast_appearances() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let icon_root = tempdir().unwrap();
        let mut production = Renderer::with_main_thread_marker(
            marker,
            IconAssetStore::new(icon_root.path().join("production")),
            RuntimePresentation::default(),
        );
        let mut development = Renderer::with_main_thread_marker(
            marker,
            IconAssetStore::new(icon_root.path().join("development")),
            development_presentation(),
        );
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let scene = scene_with_lines(&["C 42%"], &["R 68%"]);
        let appearances = [
            unsafe { objc2_app_kit::NSAppearanceNameAqua },
            unsafe { objc2_app_kit::NSAppearanceNameDarkAqua },
            unsafe { objc2_app_kit::NSAppearanceNameAccessibilityHighContrastAqua },
            unsafe { objc2_app_kit::NSAppearanceNameAccessibilityHighContrastDarkAqua },
        ];

        for name in appearances {
            let appearance = NSAppearance::appearanceNamed(name).unwrap();
            let production_output =
                production.render(RenderSlot::Status, &scene, &typography, &appearance);
            let development_output =
                development.render(RenderSlot::Status, &scene, &typography, &appearance);

            assert!(development_output.image.size().width > production_output.image.size().width);
        }
    }

    #[derive(Default)]
    struct FakeStatusTarget {
        accessibility_label: std::cell::RefCell<Option<String>>,
        tooltip: std::cell::RefCell<Option<String>>,
        image_width: Cell<Option<f64>>,
    }

    impl StatusMetadataTarget for FakeStatusTarget {
        fn set_accessibility_label(&self, value: &str) {
            self.accessibility_label.replace(Some(value.to_owned()));
        }

        fn set_tooltip(&self, value: &str) {
            self.tooltip.replace(Some(value.to_owned()));
        }
    }

    impl StatusRenderTarget for FakeStatusTarget {
        fn set_status_image(&self, image: &NSImage) {
            self.image_width.set(Some(image.size().width));
        }
    }

    #[test]
    fn status_rendering_uses_explicit_appearance_without_a_status_button_source() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let scene = scene_with_color(SegmentColor::Semantic(SemanticColor::Warning));
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();
        let target = FakeStatusTarget::default();

        renderer.apply_status(&target, &scene, &typography, &aqua);

        assert!(target.image_width.get().is_some_and(|width| width > 0.0));
        assert_eq!(
            target.accessibility_label.borrow().as_deref(),
            Some(scene.accessibility_label.as_str())
        );
    }

    #[test]
    fn native_renderer_contract_runs_only_with_a_main_thread_marker() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let scene = IndicatorScene {
            top: vec![IndicatorRun {
                text: "C 42%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Warning),
                trailing_spacing_level: 0,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Good),
                trailing_spacing_level: 0,
            }],
            top_identifier: None,
            bottom_identifier: None,
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%, pressão de memória normal".to_owned(),
        };
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();
        let dark =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua })
                .unwrap();

        let status = renderer.render(RenderSlot::Status, &scene, &typography, &aqua);
        renderer.render(RenderSlot::PreviewLight, &scene, &typography, &aqua);
        renderer.render(RenderSlot::PreviewDark, &scene, &typography, &dark);
        renderer.render(RenderSlot::Status, &scene, &typography, &aqua);

        assert!(status.image.size().width > 0.0);
        assert_eq!(renderer.cache.slots.entries.iter().flatten().count(), 3);

        let button = NSStatusBarButton::new(marker);
        renderer.apply_status(&*button, &scene, &typography, &aqua);
        assert_eq!(
            button.accessibilityLabel().unwrap().to_string(),
            scene.accessibility_label
        );
        assert_eq!(
            button.toolTip().unwrap().to_string(),
            scene.accessibility_label
        );
    }

    #[test]
    fn every_macos_14_allowlisted_symbol_also_resolves_on_the_current_host() {
        let Some(_marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit symbol validation requires a main-thread test marker");
            return;
        };
        let color = NSColor::labelColor();
        for name in SystemSymbolName::curated_names() {
            let name = SystemSymbolName::new(name).unwrap();
            assert!(
                create_system_symbol_image(&name, &color, 12.0, FontWeight::Medium).is_some(),
                "macOS 14 allowlisted SF Symbol {} must resolve on the current host",
                name.as_str()
            );
        }
    }

    #[test]
    fn equal_typography_across_scenes_resolves_font_measurer_and_attributes_once() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let mut typography =
            statlet::indicator_preferences::IndicatorPreferences::default().typography;
        typography.size = statlet::indicator_preferences::FontSize::try_from(13).unwrap();
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();
        let before = renderer.construction_stats();

        renderer.render(
            RenderSlot::Status,
            &scene_with_lines(&["C 42%"], &["R 68%"]),
            &typography,
            &aqua,
        );
        renderer.render(
            RenderSlot::PreviewLight,
            &scene_with_lines(&["C 43%"], &["R 69%"]),
            &typography,
            &aqua,
        );

        let after = renderer.construction_stats();
        assert_eq!(after.font_resolutions - before.font_resolutions, 1);
        assert_eq!(after.measurers - before.measurers, 1);
        assert_eq!(after.attribute_sets - before.attribute_sets, 1);
    }

    #[test]
    fn semantic_invalidation_preserves_fixed_srgb_image_and_typography_cache() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let fixed = SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap());
        let scene = scene_with_color(fixed);
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();
        let dark =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua })
                .unwrap();

        let first = renderer.render(RenderSlot::Status, &scene, &typography, &aqua);
        let before = renderer.construction_stats();
        renderer.invalidate_semantic_colors();
        let second = renderer.render(RenderSlot::Status, &scene, &typography, &dark);
        let after = renderer.construction_stats();

        assert!(std::ptr::eq(&*first.image, &*second.image));
        assert_eq!(after.font_resolutions, before.font_resolutions);
        assert_eq!(after.measurers, before.measurers);
        assert_eq!(after.attribute_sets, before.attribute_sets);
        assert_eq!(after.images, before.images);
    }

    #[test]
    fn semantic_invalidation_rebuilds_semantic_paint_but_not_typography() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let scene = scene_with_color(SegmentColor::Semantic(SemanticColor::Warning));
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();
        let dark =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua })
                .unwrap();

        renderer.render(RenderSlot::Status, &scene, &typography, &aqua);
        let before = renderer.construction_stats();
        renderer.invalidate_semantic_colors();
        renderer.render(RenderSlot::Status, &scene, &typography, &dark);
        let after = renderer.construction_stats();

        assert_eq!(after.font_resolutions, before.font_resolutions);
        assert_eq!(after.measurers, before.measurers);
        assert!(after.attribute_sets > before.attribute_sets);
        assert_eq!(after.images, before.images + 1);
    }

    #[test]
    fn attribute_cache_stays_bounded_during_intermediate_color_changes() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();

        for red in 0..=40 {
            let color =
                SegmentColor::Srgb(SrgbColor::parse_hex(&format!("#{red:02X}2850")).unwrap());
            renderer.render(
                RenderSlot::Status,
                &scene_with_color(color),
                &typography,
                &aqua,
            );
        }

        assert!(renderer.attribute_cache_len() <= 16);
    }

    #[test]
    fn font_refresh_invalidates_font_measurer_attributes_layout_and_images() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = production_renderer(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let scene = scene_with_lines(&["C 42%"], &["R 68%"]);
        let aqua =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }).unwrap();

        renderer.render(RenderSlot::Status, &scene, &typography, &aqua);
        let before = renderer.construction_stats();
        renderer.refresh_fonts();
        renderer.render(RenderSlot::Status, &scene, &typography, &aqua);
        let after = renderer.construction_stats();

        assert_eq!(after.font_resolutions, before.font_resolutions + 1);
        assert_eq!(after.measurers, before.measurers + 1);
        assert!(after.attribute_sets > before.attribute_sets);
        assert_eq!(after.images, before.images + 1);
    }

    #[test]
    fn label_detection_accepts_split_and_consolidated_runs_but_rejects_hidden_labels() {
        let split = scene_with_lines(&["C ", "42%"], &["R ", "68%"]);
        let consolidated = scene_with_lines(&["C 42%"], &["R 68%"]);
        let hidden = scene_with_lines(&["42%"], &["68%"]);

        assert!(label_prefix(&split.top).is_some());
        assert!(label_prefix(&consolidated.top).is_some());
        assert!(label_prefix(&hidden.top).is_none());
    }

    #[test]
    fn identical_visual_identity_reuses_the_painted_image() {
        let scene = scene_with_lines(&["C 42%"], &["R 68%"]);
        let layout_key = layout_key("Menlo", true);
        let key = paint_key(
            &scene,
            layout_key.clone(),
            "NSAppearanceNameAqua".to_owned(),
        );
        let layout = measure_stable_layout(&CountingMeasurer::new(), true, 40.0);
        let mut cache = SurfaceCache::default();
        cache.replace(
            RenderSlot::Status,
            layout_key,
            layout,
            key.clone(),
            TestEntry(7),
        );

        assert_eq!(
            cache.reused_image(RenderSlot::Status, &key),
            Some(TestEntry(7))
        );
    }

    #[test]
    fn percentage_text_change_invalidates_the_painted_image() {
        let before = scene_with_lines(&["C 42%"], &["R 68%"]);
        let after = scene_with_lines(&["C 43%"], &["R 68%"]);
        let layout = layout_key("Menlo", true);
        let cached = paint_key(&before, layout.clone(), "NSAppearanceNameAqua".to_owned());
        let requested = paint_key(&after, layout, "NSAppearanceNameAqua".to_owned());
        let mut cache = SurfaceCache::default();
        cache_test_surface(&mut cache, RenderSlot::Status, &before, TestEntry(7));

        assert_ne!(cached, requested);
        assert_eq!(cache.reused_image(RenderSlot::Status, &requested), None);
    }

    #[test]
    fn identifier_visual_change_invalidates_the_painted_image() {
        let mut before = scene_with_lines(&["42%"], &["R ", "68%"]);
        before.top_identifier = Some(MetricIdentifierVisual::SystemSymbol {
            name: SystemSymbolName::new("cpu").unwrap(),
            color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".to_owned(),
        });
        let mut after = before.clone();
        after.top_identifier = Some(MetricIdentifierVisual::SystemSymbol {
            name: SystemSymbolName::new("waveform.path.ecg").unwrap(),
            color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".to_owned(),
        });
        let layout = layout_key("Menlo", true);

        let cached = paint_key(&before, layout.clone(), "NSAppearanceNameAqua".to_owned());
        let requested = paint_key(&after, layout, "NSAppearanceNameAqua".to_owned());

        assert_ne!(cached, requested);
    }

    #[test]
    fn png_fallback_color_change_invalidates_the_painted_image() {
        let mut before = scene_with_lines(&["42%"], &["R ", "68%"]);
        before.top_identifier = Some(MetricIdentifierVisual::Png {
            metric: MetricKind::Cpu,
            metadata: PngIconMetadata::new("cpu.png", 24, 24, 100).unwrap(),
            fallback_color: SegmentColor::Semantic(SemanticColor::Neutral),
            fallback_text: "C ".to_owned(),
        });
        let mut after = before.clone();
        if let Some(MetricIdentifierVisual::Png { fallback_color, .. }) =
            after.top_identifier.as_mut()
        {
            *fallback_color = SegmentColor::Semantic(SemanticColor::Warning);
        }
        let layout = layout_key("Menlo", true);

        let cached = paint_key(&before, layout.clone(), "NSAppearanceNameAqua".to_owned());
        let requested = paint_key(&after, layout, "NSAppearanceNameAqua".to_owned());

        assert_ne!(cached, requested);
    }

    #[test]
    fn badge_text_change_invalidates_even_when_its_color_is_unchanged() {
        let mut warning = scene_with_lines(&["C 42%"], &["R 68%"]);
        warning.disk_badge = Some(IndicatorRun {
            text: " !".to_owned(),
            color: SegmentColor::Semantic(SemanticColor::DiskWarning),
            trailing_spacing_level: 0,
        });
        let mut changed = warning.clone();
        changed.disk_badge.as_mut().unwrap().text = " ?".to_owned();
        let layout = layout_key("Menlo", true);
        let cached = paint_key(&warning, layout.clone(), "NSAppearanceNameAqua".to_owned());
        let requested = paint_key(&changed, layout, "NSAppearanceNameAqua".to_owned());
        let mut cache = SurfaceCache::default();
        cache_test_surface(&mut cache, RenderSlot::Status, &warning, TestEntry(7));

        assert_ne!(cached, requested);
        assert_eq!(cache.reused_image(RenderSlot::Status, &requested), None);
    }

    fn cache_test_surface(
        cache: &mut SurfaceCache<TestEntry>,
        slot: RenderSlot,
        scene: &IndicatorScene,
        image: TestEntry,
    ) {
        let layout_key = layout_key("Menlo", true);
        let layout = measure_stable_layout(&CountingMeasurer::new(), true, 40.0);
        let paint_key = paint_key(scene, layout_key.clone(), "NSAppearanceNameAqua".to_owned());
        cache.replace(slot, layout_key, layout, paint_key, image);
    }

    fn scene_with_lines(top: &[&str], bottom: &[&str]) -> IndicatorScene {
        let runs = |texts: &[&str]| {
            texts
                .iter()
                .map(|text| IndicatorRun {
                    text: (*text).to_owned(),
                    color: SegmentColor::Semantic(SemanticColor::Neutral),
                    trailing_spacing_level: 0,
                })
                .collect()
        };
        IndicatorScene {
            top: runs(top),
            bottom: runs(bottom),
            top_identifier: None,
            bottom_identifier: None,
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%".to_owned(),
        }
    }

    fn scene_with_color(color: SegmentColor) -> IndicatorScene {
        IndicatorScene {
            top: vec![IndicatorRun {
                text: "C 42%".to_owned(),
                color,
                trailing_spacing_level: 0,
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".to_owned(),
                color,
                trailing_spacing_level: 0,
            }],
            top_identifier: None,
            bottom_identifier: None,
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%".to_owned(),
        }
    }
}
