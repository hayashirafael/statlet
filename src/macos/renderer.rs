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
    NSAccessibility, NSAppearance, NSAppearanceCustomization, NSApplication,
    NSAttributedStringNSStringDrawing, NSColor, NSColorSpace, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSImage, NSStatusBarButton, NSView,
};
use objc2_foundation::{NSDictionary, NSMutableAttributedString, NSPoint, NSSize, NSString};

use statlet::indicator::{
    measure_stable_layout, IndicatorRun, IndicatorScene, LayoutDiagnostics, SegmentColor,
    SemanticColor, StableLayout, TextMeasurer,
};
use statlet::indicator_preferences::{FontWeight, TypographyPreferences};

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
    labels_visible: bool,
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

trait StatusMetadataTarget {
    fn set_accessibility_label(&self, value: &str);
    fn set_tooltip(&self, value: &str);
}

impl StatusMetadataTarget for NSStatusBarButton {
    fn set_accessibility_label(&self, value: &str) {
        self.setAccessibilityLabel(Some(&NSString::from_str(value)));
    }

    fn set_tooltip(&self, value: &str) {
        self.setToolTip(Some(&NSString::from_str(value)));
    }
}

fn apply_status_metadata(target: &impl StatusMetadataTarget, scene: &IndicatorScene) {
    target.set_accessibility_label(&scene.accessibility_label);
    target.set_tooltip(&scene.accessibility_label);
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
    appearance: String,
    top: Vec<RunIdentity>,
    bottom: Vec<RunIdentity>,
    badge: Option<RunIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RunIdentity {
    text: String,
    paint: PaintIdentity,
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
    PaintKey {
        layout,
        top: run_identities(&scene.top, &appearance),
        bottom: run_identities(&scene.bottom, &appearance),
        badge: scene
            .disk_badge
            .as_ref()
            .map(|run| run_identity(run, &appearance)),
        appearance,
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
}

pub struct RenderOutput {
    pub image: Retained<NSImage>,
    pub layout: LayoutDiagnostics,
    pub font: FontResolution,
}

pub struct PreviewImages {
    pub light: Retained<NSImage>,
    pub dark: Retained<NSImage>,
}

fn labels_visible(scene: &IndicatorScene) -> bool {
    line_has_label(&scene.top, "C") || line_has_label(&scene.bottom, "R")
}

fn line_has_label(runs: &[IndicatorRun], label: &str) -> bool {
    let text = runs.iter().map(|run| run.text.as_str()).collect::<String>();
    text.strip_prefix(label)
        .and_then(|remainder| remainder.chars().next())
        .is_some_and(char::is_whitespace)
}

pub struct Renderer {
    font_catalog: FontCatalog,
    cache: SurfaceCache<Retained<NSImage>>,
    default_width: f64,
}

impl Renderer {
    pub fn new() -> Self {
        let marker = MainThreadMarker::new().expect("Renderer must be created on the main thread");
        Self::with_main_thread_marker(marker)
    }

    pub fn with_main_thread_marker(marker: MainThreadMarker) -> Self {
        let font_catalog = FontCatalog::new(marker);
        let default_typography =
            statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let default_font = font_catalog.resolve(&default_typography);
        let default_measurer = FontTextMeasurer::new(&default_font.font);
        let default_width =
            measure_stable_layout(&default_measurer, true, f64::INFINITY).base_width();
        Self {
            font_catalog,
            cache: SurfaceCache::default(),
            default_width,
        }
    }

    pub fn render(
        &mut self,
        slot: RenderSlot,
        scene: &IndicatorScene,
        typography: &TypographyPreferences,
        appearance: &NSAppearance,
    ) -> RenderOutput {
        let font = self.font_catalog.resolve(typography);
        let labels_visible = labels_visible(scene);
        let layout_key = LayoutKey {
            resolved_family: font.resolved_family.clone(),
            size: typography.size.points(),
            weight: typography.weight,
            labels_visible,
        };
        let measurer = FontTextMeasurer::new(&font.font);
        let layout = self.cache.resolve_layout(slot, &layout_key, || {
            measure_stable_layout(&measurer, labels_visible, self.default_width)
        });
        let appearance_name = appearance.name().to_string();
        let paint_key = paint_key(scene, layout_key.clone(), appearance_name);
        if let Some(image) = self.cache.reused_image(slot, &paint_key) {
            return RenderOutput {
                image,
                layout: layout.diagnostics,
                font,
            };
        }
        let image = draw_image(scene, &font.font, layout, appearance);

        self.cache
            .replace(slot, layout_key, layout, paint_key, image.clone());

        RenderOutput {
            image,
            layout: layout.diagnostics,
            font,
        }
    }

    pub fn apply_status(
        &mut self,
        button: &NSStatusBarButton,
        scene: &IndicatorScene,
        typography: &TypographyPreferences,
    ) -> LayoutDiagnostics {
        let appearance = button.effectiveAppearance();
        let output = self.render(RenderSlot::Status, scene, typography, &appearance);
        button.setImage(Some(&output.image));
        apply_status_metadata(button, scene);
        output.layout
    }

    pub fn refresh_fonts(&mut self) {
        self.font_catalog.refresh();
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.cache.clear();
    }
}

struct FontTextMeasurer {
    font: Retained<NSFont>,
    attributes: Retained<NSDictionary<NSString, AnyObject>>,
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

fn draw_image(
    scene: &IndicatorScene,
    font: &NSFont,
    layout: StableLayout,
    appearance: &NSAppearance,
) -> Retained<NSImage> {
    let rendered = RefCell::new(None);
    let draw = StackBlock::new(|| {
        let badge = scene.disk_badge.as_ref().map(|run| run.text.as_str());
        let width = layout.width_for_badge(badge).ceil();
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
        let measurer = FontTextMeasurer::new(font);

        #[allow(deprecated)]
        {
            image.lockFocus();
            draw_metric_line(
                &scene.bottom,
                margin - descent,
                font,
                &measurer,
                layout,
                layout.ram_width,
            );
            draw_metric_line(
                &scene.top,
                margin + cap_height + LINE_GAP - descent,
                font,
                &measurer,
                layout,
                layout.cpu_width,
            );
            if let Some(badge) = &scene.disk_badge {
                attributed_scene_run(font, badge).drawAtPoint(NSPoint {
                    x: layout.base_width(),
                    y: margin + cap_height + LINE_GAP - descent,
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

fn draw_metric_line(
    runs: &[IndicatorRun],
    y: f64,
    font: &NSFont,
    measurer: &impl TextMeasurer,
    layout: StableLayout,
    line_width: f64,
) {
    match runs {
        [value] => attributed_scene_run(font, value).drawAtPoint(NSPoint {
            x: layout.value_origin(measurer, line_width, &value.text),
            y,
        }),
        [label, value] => {
            attributed_scene_run(font, label).drawAtPoint(NSPoint { x: 0.0, y });
            attributed_scene_run(font, value).drawAtPoint(NSPoint {
                x: layout.value_origin(measurer, line_width, &value.text),
                y,
            });
        }
        _ => attributed_scene_line(font, runs.iter()).drawAtPoint(NSPoint { x: 0.0, y }),
    }
}

fn attributed_scene_run(
    font: &NSFont,
    run: &IndicatorRun,
) -> Retained<objc2_foundation::NSAttributedString> {
    let color = resolve_color(run.color);
    let attributes = NSDictionary::from_retained_objects(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[
            Retained::into_super(Retained::into_super(font.retain())),
            Retained::into_super(Retained::into_super(color)),
        ],
    );
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
) -> Retained<NSMutableAttributedString> {
    let line = NSMutableAttributedString::new();
    for run in runs {
        line.appendAttributedString(&attributed_scene_run(font, run));
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
            .top
            .iter()
            .chain(&scene.bottom)
            .chain(scene.disk_badge.iter())
            .map(|run| resolved_srgb_components(&resolve_color(run.color)))
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

    use statlet::indicator::{
        has_low_text_contrast, measure_stable_layout, IndicatorRun, IndicatorScene,
        PreviewBackground, SegmentColor, SemanticColor, TextMeasurer,
    };
    use statlet::indicator_preferences::{FontWeight, SrgbColor};

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestEntry(u8);

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

    fn layout_key(family: &str, labels_visible: bool) -> LayoutKey {
        LayoutKey {
            resolved_family: family.to_owned(),
            size: 12,
            weight: FontWeight::Medium,
            labels_visible,
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
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Warning),
            }],
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
    fn accessible_status_metadata_is_applied_to_the_target() {
        let scene = IndicatorScene {
            top: vec![IndicatorRun {
                text: "42%".to_owned(),
                color: SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap()),
            }],
            bottom: vec![IndicatorRun {
                text: "68%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Good),
            }],
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%, pressão de memória normal".to_owned(),
        };

        let target = FakeStatusTarget::default();

        apply_status_metadata(&target, &scene);

        assert_eq!(
            target.accessibility_label.borrow().as_deref(),
            Some(scene.accessibility_label.as_str())
        );
        assert_eq!(
            target.tooltip.borrow().as_deref(),
            Some(scene.accessibility_label.as_str())
        );
    }

    #[derive(Default)]
    struct FakeStatusTarget {
        accessibility_label: std::cell::RefCell<Option<String>>,
        tooltip: std::cell::RefCell<Option<String>>,
    }

    impl StatusMetadataTarget for FakeStatusTarget {
        fn set_accessibility_label(&self, value: &str) {
            self.accessibility_label.replace(Some(value.to_owned()));
        }

        fn set_tooltip(&self, value: &str) {
            self.tooltip.replace(Some(value.to_owned()));
        }
    }

    #[test]
    fn native_renderer_contract_runs_only_with_a_main_thread_marker() {
        let Some(marker) = MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit rendering requires a main-thread test marker");
            return;
        };
        let mut renderer = Renderer::with_main_thread_marker(marker);
        let typography = statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let scene = IndicatorScene {
            top: vec![IndicatorRun {
                text: "C 42%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Warning),
            }],
            bottom: vec![IndicatorRun {
                text: "R 68%".to_owned(),
                color: SegmentColor::Semantic(SemanticColor::Good),
            }],
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
        renderer.apply_status(&button, &scene, &typography);
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
    fn label_detection_accepts_split_and_consolidated_runs_but_rejects_hidden_labels() {
        let split = scene_with_lines(&["C ", "42%"], &["R ", "68%"]);
        let consolidated = scene_with_lines(&["C 42%"], &["R 68%"]);
        let hidden = scene_with_lines(&["42%"], &["68%"]);

        assert!(labels_visible(&split));
        assert!(labels_visible(&consolidated));
        assert!(!labels_visible(&hidden));
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
    fn badge_text_change_invalidates_even_when_its_color_is_unchanged() {
        let mut warning = scene_with_lines(&["C 42%"], &["R 68%"]);
        warning.disk_badge = Some(IndicatorRun {
            text: " !".to_owned(),
            color: SegmentColor::Semantic(SemanticColor::DiskWarning),
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
                })
                .collect()
        };
        IndicatorScene {
            top: runs(top),
            bottom: runs(bottom),
            disk_badge: None,
            accessibility_label: "CPU 42%, RAM 68%".to_owned(),
        }
    }
}
