//! Two-line status-item renderer.
//!
//! Derived and modified from featherbar commit 90ab504, Apache-2.0:
//! https://github.com/nim444/featherbar/tree/90ab504b025db15665ce5d97b8ae4d4cdeb47dc3

// Task 6 exposes the shared renderer APIs before Task 7 replaces the legacy runtime call site.
#![allow(dead_code)]

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

use statlet::core::{DiskBadge, MetricContent, MetricSeverity, StatusContent};
use statlet::indicator::{
    measure_stable_layout, IndicatorRun, IndicatorScene, LayoutDiagnostics, SegmentColor,
    SemanticColor, StableLayout, TextMeasurer,
};
use statlet::indicator_preferences::{FontWeight, TypographyPreferences};

use super::fonts::{FontCatalog, FontResolution};

const FONT_SIZE: f64 = 12.0;
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

    fn len(&self) -> usize {
        self.entries.iter().flatten().count()
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

#[cfg(test)]
#[derive(Clone)]
struct CachedLayout {
    key: LayoutKey,
    layout: StableLayout,
}

#[cfg(test)]
#[derive(Default)]
struct LayoutSlots {
    slots: SlotMap<CachedLayout>,
}

#[cfg(test)]
impl LayoutSlots {
    fn resolve(
        &mut self,
        slot: RenderSlot,
        key: LayoutKey,
        measure: impl FnOnce() -> StableLayout,
    ) -> StableLayout {
        let layout = resolve_layout(
            self.slots
                .get(slot)
                .map(|cached| (&cached.key, cached.layout)),
            &key,
            measure,
        );
        self.slots.replace(slot, CachedLayout { key, layout });
        layout
    }
}

fn resolve_layout(
    cached: Option<(&LayoutKey, StableLayout)>,
    requested: &LayoutKey,
    measure: impl FnOnce() -> StableLayout,
) -> StableLayout {
    match cached {
        Some((key, layout)) if key == requested => layout,
        _ => measure(),
    }
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
    match color {
        SegmentColor::Srgb(color) => PaintIdentity::Srgb(color.components()),
        SegmentColor::Semantic(color) => PaintIdentity::Semantic {
            color,
            appearance: appearance.to_owned(),
        },
    }
}

struct StatusMetadata {
    accessibility_label: String,
    tooltip: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaintKey {
    layout: LayoutKey,
    appearance: String,
    colors: Vec<PaintIdentity>,
}

struct SlotCache {
    layout_key: LayoutKey,
    layout: StableLayout,
    paint_key: PaintKey,
    image: Retained<NSImage>,
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

fn status_metadata(scene: &statlet::indicator::IndicatorScene) -> StatusMetadata {
    StatusMetadata {
        accessibility_label: scene.accessibility_label.clone(),
        tooltip: scene.accessibility_label.clone(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Level {
    Neutral,
    Good,
    Warning,
    Critical,
    DiskWarning,
    DiskError,
}

struct Segment {
    text: String,
    level: Level,
}

pub struct Renderer {
    attributes: [Retained<NSDictionary<NSString, AnyObject>>; 6],
    top_y: f64,
    bottom_y: f64,
    font_catalog: FontCatalog,
    slots: SlotMap<SlotCache>,
    default_width: f64,
}

impl Renderer {
    pub fn new() -> Self {
        let marker = MainThreadMarker::new().expect("Renderer must be created on the main thread");
        Self::with_main_thread_marker(marker)
    }

    pub fn with_main_thread_marker(marker: MainThreadMarker) -> Self {
        let font = NSFont::monospacedSystemFontOfSize_weight(FONT_SIZE, unsafe {
            objc2_app_kit::NSFontWeightMedium
        });
        let attributes = [
            Level::Neutral,
            Level::Good,
            Level::Warning,
            Level::Critical,
            Level::DiskWarning,
            Level::DiskError,
        ]
        .map(|level| {
            NSDictionary::from_retained_objects(
                &[unsafe { NSFontAttributeName }, unsafe {
                    NSForegroundColorAttributeName
                }],
                &[
                    Retained::into_super(Retained::into_super(font.retain())),
                    Retained::into_super(Retained::into_super(color(level))),
                ],
            )
        });

        let cap_height = font.capHeight();
        let descent = -font.descender();
        let margin = (HEIGHT - 2.0 * cap_height - LINE_GAP) / 2.0;
        let font_catalog = FontCatalog::new(marker);
        let default_typography =
            statlet::indicator_preferences::IndicatorPreferences::default().typography;
        let default_font = font_catalog.resolve(&default_typography);
        let default_measurer = FontTextMeasurer::new(&default_font.font);
        let default_width =
            measure_stable_layout(&default_measurer, true, f64::INFINITY).base_width();
        Self {
            attributes,
            bottom_y: margin - descent,
            top_y: margin + cap_height + LINE_GAP - descent,
            font_catalog,
            slots: SlotMap::default(),
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
        let labels_visible = scene.top.len() > 1 || scene.bottom.len() > 1;
        let layout_key = LayoutKey {
            resolved_family: font.resolved_family.clone(),
            size: typography.size.points(),
            weight: typography.weight,
            labels_visible,
        };
        let measurer = FontTextMeasurer::new(&font.font);
        let layout = resolve_layout(
            self.slots
                .get(slot)
                .map(|cached| (&cached.layout_key, cached.layout)),
            &layout_key,
            || measure_stable_layout(&measurer, labels_visible, self.default_width),
        );
        let appearance_name = appearance.name().to_string();
        let paint_key = PaintKey {
            layout: layout_key.clone(),
            appearance: appearance_name.clone(),
            colors: scene_colors(scene)
                .map(|color| paint_identity(color, &appearance_name))
                .collect(),
        };
        let image = draw_image(scene, &font.font, layout, appearance);

        self.slots.replace(
            slot,
            SlotCache {
                layout_key,
                layout,
                paint_key,
                image: image.clone(),
            },
        );

        RenderOutput {
            image,
            layout: layout.diagnostics,
            font,
        }
    }

    pub fn render_previews(
        &mut self,
        light_scene: &IndicatorScene,
        dark_scene: &IndicatorScene,
        typography: &TypographyPreferences,
    ) -> PreviewImages {
        let light_appearance =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua })
                .expect("Aqua appearance is available on macOS");
        let dark_appearance =
            NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua })
                .expect("Dark Aqua appearance is available on macOS");
        PreviewImages {
            light: self
                .render(
                    RenderSlot::PreviewLight,
                    light_scene,
                    typography,
                    &light_appearance,
                )
                .image,
            dark: self
                .render(
                    RenderSlot::PreviewDark,
                    dark_scene,
                    typography,
                    &dark_appearance,
                )
                .image,
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
        let metadata = status_metadata(scene);
        button.setImage(Some(&output.image));
        button.setAccessibilityLabel(Some(&NSString::from_str(&metadata.accessibility_label)));
        button.setToolTip(Some(&NSString::from_str(&metadata.tooltip)));
        output.layout
    }

    pub fn cached_slot_count(&self) -> usize {
        self.slots.len()
    }

    pub fn font_families(&self) -> &[String] {
        self.font_catalog.families()
    }

    pub fn refresh_fonts(&mut self) {
        self.font_catalog.refresh();
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.slots.clear();
    }

    pub fn set_status(&self, button: &NSStatusBarButton, status: &StatusContent) {
        let top_segments = segments(&status.cpu);
        let top = if let Some(badge) = disk_badge_segment(status.disk_badge) {
            let [label, value] = top_segments;
            self.attributed_line(&[label, value, badge])
        } else {
            self.attributed_line(&top_segments)
        };
        let bottom = self.attributed_line(&segments(&status.ram));
        let width = top.size().width.max(bottom.size().width).ceil();
        let image = NSImage::initWithSize(
            NSImage::alloc(),
            NSSize {
                width,
                height: HEIGHT,
            },
        );

        #[allow(deprecated)]
        {
            image.lockFocus();
            bottom.drawAtPoint(NSPoint {
                x: 0.0,
                y: self.bottom_y,
            });
            top.drawAtPoint(NSPoint {
                x: 0.0,
                y: self.top_y,
            });
            image.unlockFocus();
        }

        button.setImage(Some(&image));
        button.setAccessibilityLabel(Some(&NSString::from_str(&status.accessibility_label)));
        button.setToolTip(Some(&NSString::from_str(&status.accessibility_label)));
    }

    fn attributed_line(&self, segments: &[Segment]) -> Retained<NSMutableAttributedString> {
        let line = NSMutableAttributedString::new();
        for segment in segments {
            let run = unsafe {
                objc2_foundation::NSAttributedString::new_with_attributes(
                    &NSString::from_str(&segment.text),
                    &self.attributes[segment.level as usize],
                )
            };
            line.appendAttributedString(&run);
        }
        line
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

fn scene_colors(scene: &IndicatorScene) -> impl Iterator<Item = SegmentColor> + '_ {
    scene
        .top
        .iter()
        .chain(&scene.bottom)
        .chain(scene.disk_badge.iter())
        .map(|run| run.color)
}

fn draw_image(
    scene: &IndicatorScene,
    font: &NSFont,
    layout: StableLayout,
    appearance: &NSAppearance,
) -> Retained<NSImage> {
    let rendered = RefCell::new(None);
    let draw = StackBlock::new(|| {
        let top = attributed_scene_line(font, scene.top.iter().chain(scene.disk_badge.iter()));
        let bottom = attributed_scene_line(font, scene.bottom.iter());
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

        #[allow(deprecated)]
        {
            image.lockFocus();
            bottom.drawAtPoint(NSPoint {
                x: 0.0,
                y: margin - descent,
            });
            top.drawAtPoint(NSPoint {
                x: 0.0,
                y: margin + cap_height + LINE_GAP - descent,
            });
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

fn attributed_scene_line<'a>(
    font: &NSFont,
    runs: impl Iterator<Item = &'a IndicatorRun>,
) -> Retained<NSMutableAttributedString> {
    let line = NSMutableAttributedString::new();
    for run in runs {
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
        let attributed = unsafe {
            objc2_foundation::NSAttributedString::new_with_attributes(
                &NSString::from_str(&run.text),
                &attributes,
            )
        };
        line.appendAttributedString(&attributed);
    }
    line
}

fn resolve_color(color: SegmentColor) -> Retained<NSColor> {
    match color {
        SegmentColor::Semantic(color) => semantic_color(color),
        SegmentColor::Srgb(color) => {
            let [red, green, blue] = color
                .components()
                .map(|component| f64::from(component) / 255.0);
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0);
            color
                .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
                .unwrap_or(color)
        }
    }
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

fn segments(metric: &MetricContent) -> [Segment; 2] {
    [
        Segment {
            text: metric.label.to_owned(),
            level: Level::Neutral,
        },
        Segment {
            text: format!("{:>3}%", metric.percent),
            level: match metric.severity {
                MetricSeverity::Good => Level::Good,
                MetricSeverity::Warning => Level::Warning,
                MetricSeverity::Critical => Level::Critical,
            },
        },
    ]
}

fn disk_badge_segment(badge: Option<DiskBadge>) -> Option<Segment> {
    badge.map(|badge| match badge {
        DiskBadge::Warning => Segment {
            text: " !".to_owned(),
            level: Level::DiskWarning,
        },
        DiskBadge::Error => Segment {
            text: " ×".to_owned(),
            level: Level::DiskError,
        },
    })
}

fn color(level: Level) -> Retained<NSColor> {
    match level {
        Level::Neutral => NSColor::labelColor(),
        Level::Good => NSColor::systemGreenColor(),
        Level::Warning => NSColor::systemOrangeColor(),
        Level::Critical => NSColor::systemRedColor(),
        Level::DiskWarning => NSColor::systemYellowColor(),
        Level::DiskError => NSColor::systemRedColor(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use statlet::indicator::{
        measure_stable_layout, IndicatorRun, IndicatorScene, SegmentColor, SemanticColor,
        TextMeasurer,
    };
    use statlet::indicator_preferences::{FontWeight, SrgbColor};

    use super::*;

    fn cpu_metric() -> MetricContent {
        MetricContent {
            label: "C",
            percent: 42,
            severity: MetricSeverity::Warning,
        }
    }

    #[test]
    fn disk_warning_appends_a_symbolic_yellow_segment() {
        let segment = disk_badge_segment(Some(DiskBadge::Warning)).unwrap();

        assert_eq!(segment.text, " !");
        assert_eq!(segment.level, Level::DiskWarning);
    }

    #[test]
    fn no_disk_badge_preserves_the_compact_cpu_line() {
        let segments = segments(&cpu_metric());

        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>(),
            "C 42%"
        );
        assert!(disk_badge_segment(None).is_none());
    }

    #[test]
    fn legacy_renderer_keeps_values_padded_to_three_digits() {
        let cases = [(0, "  0%"), (9, "  9%"), (10, " 10%"), (100, "100%")];

        for (percent, expected) in cases {
            let mut metric = cpu_metric();
            metric.percent = percent;

            assert_eq!(segments(&metric)[1].text, expected);
        }
    }

    #[test]
    fn mole_error_appends_a_symbolic_red_segment() {
        let segment = disk_badge_segment(Some(DiskBadge::Error)).unwrap();

        assert_eq!(segment.text, " ×");
        assert_eq!(segment.level, Level::DiskError);
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestEntry(u8);

    #[test]
    fn rerender_replaces_one_entry_per_surface_instead_of_accumulating() {
        let mut slots = SlotMap::default();

        slots.replace(RenderSlot::Status, TestEntry(1));
        slots.replace(RenderSlot::PreviewLight, TestEntry(2));
        slots.replace(RenderSlot::PreviewDark, TestEntry(3));
        slots.replace(RenderSlot::Status, TestEntry(4));

        assert_eq!(slots.len(), 3);
        assert_eq!(slots.get(RenderSlot::Status), Some(&TestEntry(4)));
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
        let mut layouts = LayoutSlots::default();
        let key = layout_key("Menlo", true);

        layouts.resolve(RenderSlot::Status, key.clone(), || {
            measure_stable_layout(&measurer, true, 40.0)
        });
        let initial_calls = measurer.calls.get();
        layouts.resolve(RenderSlot::Status, key, || {
            measure_stable_layout(&measurer, true, 40.0)
        });
        assert_eq!(measurer.calls.get(), initial_calls);

        layouts.resolve(RenderSlot::Status, layout_key("Menlo", false), || {
            measure_stable_layout(&measurer, false, 40.0)
        });
        assert!(measurer.calls.get() > initial_calls);
        let after_labels = measurer.calls.get();

        layouts.resolve(RenderSlot::Status, layout_key("Avenir Next", false), || {
            measure_stable_layout(&measurer, false, 40.0)
        });
        assert!(measurer.calls.get() > after_labels);
    }

    #[test]
    fn fixed_paint_is_srgb_while_semantic_paint_tracks_the_supplied_appearance() {
        let fixed = SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap());
        let semantic = SegmentColor::Semantic(SemanticColor::Warning);

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

    #[test]
    fn accessible_status_metadata_ignores_visible_runs_and_colors() {
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

        let metadata = status_metadata(&scene);

        assert_eq!(metadata.accessibility_label, scene.accessibility_label);
        assert_eq!(metadata.tooltip, scene.accessibility_label);
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
        assert_eq!(renderer.cached_slot_count(), 3);

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
}
