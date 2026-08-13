use std::cell::{Cell, RefCell};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey,
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityPostNotification,
    NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityPriorityKey,
    NSAccessibilityPriorityLevel, NSAccessibilityValueChangedNotification, NSApplication,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBezierPath, NSButton, NSColor,
    NSControlTextEditingDelegate, NSEvent, NSEventModifierFlags, NSFocusRingType, NSLineBreakMode,
    NSScrollView, NSSegmentSwitchTracking, NSSegmentedControl, NSTableColumn,
    NSTableColumnResizingOptions, NSTableView, NSTableViewColumnAutoresizingStyle,
    NSTableViewDataSource, NSTableViewDelegate, NSTextField, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSDate, NSDateFormatter, NSDateFormatterStyle,
    NSDictionary, NSIndexSet, NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString,
};
use statlet::core::{AppState, WindowKind};
use statlet::history::History;
use statlet::indicator::LayoutDiagnostics;
use statlet::indicator_preferences::FontFamilyPreference;
use statlet::system_usage::{
    graph_pointer_selection, history_x_position, GraphNavigation, GraphNavigationCommand,
    MemoryCompositionSegment, ProcessListStatus, ProcessRowViewModel, SurfaceObservation,
    SystemUsageAccessibilityCoordinator, SystemUsageSection, SystemUsageViewModel, UsagePoint,
};
use tao::event_loop::EventLoopProxy;

use super::environment::VisualEnvironment;
use super::renderer::PreviewImages;
use super::RuntimeEvent;

mod common;
mod free_space;
mod history;
mod preferences;

use common::{text_label, ControlTarget};
use free_space::{create_free_space_window, FreeSpaceWindow};
use history::{create_history_window, HistoryWindow};
use preferences::{get_or_create_window, PreferencesWindow};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewContrastWarnings {
    pub light: bool,
    pub dark: bool,
}

struct UsageGraphViewIvars {
    points: RefCell<Vec<UsagePoint>>,
    navigation: RefCell<GraphNavigation>,
    selected_index: Cell<Option<usize>>,
    hover_index: Cell<Option<usize>>,
    hover_normalized_x: Cell<Option<f64>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    hover_label: RefCell<Option<Retained<NSTextField>>>,
    rendered_at_unix_seconds: Cell<f64>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = UsageGraphViewIvars]
    struct UsageGraphView;

    unsafe impl NSObjectProtocol for UsageGraphView {}

    impl UsageGraphView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let accepted: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            self.setNeedsDisplay(true);
            accepted
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            let accepted: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            self.setNeedsDisplay(true);
            accepted
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let option = event
                .modifierFlags()
                .contains(NSEventModifierFlags::Option);
            let command = match (event.keyCode(), option) {
                (123, false) => Some(GraphNavigationCommand::Previous),
                (124, false) => Some(GraphNavigationCommand::Next),
                (123, true) => Some(GraphNavigationCommand::First),
                (124, true) => Some(GraphNavigationCommand::Last),
                _ => None,
            };
            let Some(command) = command else {
                unsafe {
                    let _: () = msg_send![super(self), keyDown: event];
                }
                return;
            };
            let points = self.ivars().points.borrow();
            if let Some(selection) = self
                .ivars()
                .navigation
                .borrow_mut()
                .move_selection(&points, command)
            {
                self.ivars().selected_index.set(Some(selection.index));
                let value = NSString::from_str(&selection.accessibility_value);
                unsafe { self.setAccessibilityValue(Some(&value)) };
                if selection.should_notify_accessibility {
                    unsafe {
                        NSAccessibilityPostNotification(
                            self,
                            NSAccessibilityValueChangedNotification,
                        )
                    };
                }
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let location = self.convertPoint_fromView(event.locationInWindow(), None);
            let width = (self.bounds().size.width - 4.0).max(1.0);
            let normalized_x = ((location.x - 2.0) / width).clamp(0.0, 1.0);
            self.ivars().hover_normalized_x.set(Some(normalized_x));
            self.update_hover(normalized_x);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().hover_index.set(None);
            self.ivars().hover_normalized_x.set(None);
            if let Some(label) = self.ivars().hover_label.borrow().as_ref() {
                label.setHidden(true);
            }
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let points = self.ivars().points.borrow();
            let width = (bounds.size.width - 4.0).max(1.0);
            let height = (bounds.size.height - 4.0).max(1.0);
            if let Some(window_end) = points.last().map(|point| point.observed_at) {
                if points.len() >= 2 {
                    let path = NSBezierPath::new();
                    let mut connected = false;
                    for point in points.iter() {
                        let Some(value) = point.value.filter(|value| value.is_finite()) else {
                            connected = false;
                            continue;
                        };
                        let graph_point = NSPoint::new(
                            2.0 + width * history_x_position(point.observed_at, window_end),
                            2.0 + height * value.clamp(0.0, 100.0) / 100.0,
                        );
                        if connected {
                            path.lineToPoint(graph_point);
                        } else {
                            path.moveToPoint(graph_point);
                            connected = true;
                        }
                    }
                    let increase_contrast = NSWorkspace::sharedWorkspace()
                        .accessibilityDisplayShouldIncreaseContrast();
                    path.setLineWidth(if increase_contrast { 3.0 } else { 2.0 });
                    NSColor::controlAccentColor().setStroke();
                    path.stroke();
                }

                if let Some(index) = self
                    .ivars()
                    .hover_index
                    .get()
                    .or_else(|| self.ivars().selected_index.get())
                {
                    if let Some(point) = points.get(index).filter(|point| point.value.is_some()) {
                        let marker = NSPoint::new(
                            2.0 + width * history_x_position(point.observed_at, window_end),
                            2.0
                                + height
                                    * point.value.unwrap_or_default().clamp(0.0, 100.0)
                                    / 100.0,
                        );
                        NSColor::controlAccentColor().setFill();
                        NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                            NSPoint::new(marker.x - 4.0, marker.y - 4.0),
                            NSSize::new(8.0, 8.0),
                        ))
                        .fill();
                    }
                }
            }

            let focused = self.window().and_then(|window| window.firstResponder()).is_some_and(
                |responder| unsafe { msg_send![&*responder, isEqual: self] },
            );
            if focused {
                let ring = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    NSRect::new(
                        NSPoint::new(1.0, 1.0),
                        NSSize::new(
                            (bounds.size.width - 2.0).max(1.0),
                            (bounds.size.height - 2.0).max(1.0),
                        ),
                    ),
                    4.0,
                    4.0,
                );
                ring.setLineWidth(2.0);
                NSColor::keyboardFocusIndicatorColor().setStroke();
                ring.stroke();
            }
        }
    }
);

impl UsageGraphView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(UsageGraphViewIvars {
            points: RefCell::new(Vec::new()),
            navigation: RefCell::new(GraphNavigation::new()),
            selected_index: Cell::new(None),
            hover_index: Cell::new(None),
            hover_normalized_x: Cell::new(None),
            tracking_area: RefCell::new(None),
            hover_label: RefCell::new(None),
            rendered_at_unix_seconds: Cell::new(0.0),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this.setFocusRingType(NSFocusRingType::Exterior);
        let tracking_area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
                NSTrackingAreaOptions::MouseMoved
                    | NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&*this),
                None,
            )
        };
        this.addTrackingArea(&tracking_area);
        this.ivars().tracking_area.replace(Some(tracking_area));
        let hover_label = NSTextField::labelWithString(ns_string!(""), mtm);
        hover_label.setFrame(NSRect::new(
            NSPoint::new(8.0, (frame.size.height - 24.0).max(2.0)),
            NSSize::new(220.0, 18.0),
        ));
        hover_label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
        hover_label.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(12.0)));
        hover_label.setAccessibilityElement(false);
        hover_label.setHidden(true);
        this.addSubview(&hover_label);
        this.ivars().hover_label.replace(Some(hover_label));
        this
    }

    fn apply(&self, points: &[UsagePoint], accessibility_label: &str) {
        self.ivars().points.replace(points.to_vec());
        self.ivars().rendered_at_unix_seconds.set(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        );
        if let Some(normalized_x) = self.ivars().hover_normalized_x.get() {
            self.update_hover(normalized_x);
        }
        let selection = self.ivars().navigation.borrow_mut().update_points(points);
        self.ivars()
            .selected_index
            .set(selection.as_ref().map(|selection| selection.index));
        self.setAccessibilityElement(true);
        self.setAccessibilityLabel(Some(&NSString::from_str(accessibility_label)));
        self.setAccessibilityHelp(Some(ns_string!(
            "Use as setas esquerda e direita para percorrer as amostras; Option vai ao início ou fim."
        )));
        let accessibility_value = selection.map_or_else(
            || "Nenhuma amostra válida.".to_owned(),
            |selection| selection.accessibility_value,
        );
        let value = NSString::from_str(&accessibility_value);
        unsafe { self.setAccessibilityValue(Some(&value)) };
        self.setNeedsDisplay(true);
    }

    fn update_hover(&self, normalized_x: f64) {
        let points = self.ivars().points.borrow();
        let selection = graph_pointer_selection(&points, normalized_x);
        self.ivars()
            .hover_index
            .set(selection.as_ref().map(|selection| selection.index));
        if let Some(label) = self.ivars().hover_label.borrow().as_ref() {
            if let Some(selection) = selection {
                let point = points[selection.index];
                let window_end = points
                    .last()
                    .map_or(point.observed_at, |last| last.observed_at);
                let sampled_at = self.ivars().rendered_at_unix_seconds.get();
                let sample_time =
                    sampled_at - window_end.saturating_sub(point.observed_at).as_secs_f64();
                let date = NSDate::dateWithTimeIntervalSince1970(sample_time);
                let local_time = NSDateFormatter::localizedStringFromDate_dateStyle_timeStyle(
                    &date,
                    NSDateFormatterStyle::NoStyle,
                    NSDateFormatterStyle::MediumStyle,
                );
                label.setStringValue(&NSString::from_str(&format!(
                    "{}% · {}",
                    point.value.unwrap_or_default().round() as u8,
                    local_time
                )));
                label.setHidden(false);
            } else {
                label.setHidden(true);
            }
        }
        self.setNeedsDisplay(true);
    }
}

struct MemoryCompositionViewIvars {
    fractions: RefCell<Vec<f64>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MemoryCompositionStrokeStyle {
    outline_width: f64,
    divider_width: f64,
}

fn memory_composition_stroke_style(increase_contrast: bool) -> MemoryCompositionStrokeStyle {
    if increase_contrast {
        MemoryCompositionStrokeStyle {
            outline_width: 3.0,
            divider_width: 2.0,
        }
    } else {
        MemoryCompositionStrokeStyle {
            outline_width: 1.0,
            divider_width: 1.0,
        }
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MemoryCompositionViewIvars]
    struct MemoryCompositionView;

    unsafe impl NSObjectProtocol for MemoryCompositionView {}

    impl MemoryCompositionView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let fractions = self.ivars().fractions.borrow();
            let colors = [
                NSColor::systemBlueColor(),
                NSColor::systemOrangeColor(),
                NSColor::systemPurpleColor(),
                NSColor::systemGrayColor(),
            ];
            let increase_contrast = NSWorkspace::sharedWorkspace()
                .accessibilityDisplayShouldIncreaseContrast();
            let stroke_style = memory_composition_stroke_style(increase_contrast);
            let mut x = bounds.origin.x;
            for (index, fraction) in fractions.iter().copied().enumerate() {
                let remaining = bounds.origin.x + bounds.size.width - x;
                let width = if index + 1 == fractions.len() {
                    remaining
                } else {
                    (bounds.size.width * fraction.clamp(0.0, 1.0)).min(remaining)
                };
                colors[index.min(colors.len() - 1)].setFill();
                NSBezierPath::fillRect(NSRect::new(
                    NSPoint::new(x, bounds.origin.y),
                    NSSize::new(width.max(0.0), bounds.size.height),
                ));
                x += width;
                if index + 1 < fractions.len() {
                    let divider = NSBezierPath::new();
                    divider.moveToPoint(NSPoint::new(x, bounds.origin.y));
                    divider.lineToPoint(NSPoint::new(x, bounds.origin.y + bounds.size.height));
                    divider.setLineWidth(stroke_style.divider_width);
                    NSColor::windowBackgroundColor().setStroke();
                    divider.stroke();
                }
            }
            let inset = stroke_style.outline_width / 2.0;
            let outline = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                NSRect::new(
                    NSPoint::new(bounds.origin.x + inset, bounds.origin.y + inset),
                    NSSize::new(
                        (bounds.size.width - stroke_style.outline_width).max(1.0),
                        (bounds.size.height - stroke_style.outline_width).max(1.0),
                    ),
                ),
                3.0,
                3.0,
            );
            outline.setLineWidth(stroke_style.outline_width);
            NSColor::separatorColor().setStroke();
            outline.stroke();
        }
    }
);

impl MemoryCompositionView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MemoryCompositionViewIvars {
            fractions: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn apply(&self, segments: &[MemoryCompositionSegment], accessibility_label: &str) {
        self.ivars()
            .fractions
            .replace(segments.iter().map(|segment| segment.fraction).collect());
        self.setAccessibilityElement(true);
        self.setAccessibilityLabel(Some(ns_string!("Composição da memória física")));
        let value = NSString::from_str(accessibility_label);
        unsafe { self.setAccessibilityValue(Some(&value)) };
        self.setNeedsDisplay(true);
    }
}

struct StatsTableDataSourceIvars {
    rows: RefCell<Vec<ProcessRowViewModel>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = StatsTableDataSourceIvars]
    struct StatsTableDataSource;

    unsafe impl NSObjectProtocol for StatsTableDataSource {}

    unsafe impl NSControlTextEditingDelegate for StatsTableDataSource {}

    unsafe impl NSTableViewDataSource for StatsTableDataSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table_view: &NSTableView) -> isize {
            self.ivars().rows.borrow().len() as isize
        }

        #[unsafe(method_id(tableView:objectValueForTableColumn:row:))]
        fn object_value(
            &self,
            _table_view: &NSTableView,
            table_column: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<AnyObject>> {
            usize::try_from(row)
                .ok()
                .and_then(|row| {
                    let rows = self.ivars().rows.borrow();
                    let process = rows.get(row)?;
                    let column = table_column?.identifier().to_string();
                    Some(if column == "memory" {
                        process.memory.clone()
                    } else {
                        process.name.clone()
                    })
                })
                .map(|value| Retained::into_super(Retained::into_super(NSString::from_str(&value))))
        }
    }

    unsafe impl NSTableViewDelegate for StatsTableDataSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            _table_view: &NSTableView,
            table_column: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<NSView>> {
            (|| {
                let rows = self.ivars().rows.borrow();
                let process = rows.get(usize::try_from(row).ok()?)?;
                let column = table_column?.identifier().to_string();
                let (text, accessibility_label) = process_cell_presentation(process, &column);
                let field = NSTextField::labelWithString(
                    &NSString::from_str(&text),
                    MainThreadMarker::new().expect("table cells are created on the main thread"),
                );
                field.setAccessibilityLabel(Some(&NSString::from_str(&accessibility_label)));
                Some(Retained::into_super(Retained::into_super(field)))
            })()
        }
    }
);

fn process_cell_presentation(row: &ProcessRowViewModel, column: &str) -> (String, String) {
    let text = if column == "memory" {
        row.memory.clone()
    } else {
        row.name.clone()
    };
    let accessibility_label = if column == "memory" {
        format!("Memória: {}", row.memory)
    } else {
        format!("Processo: {}", row.name)
    };
    (text, accessibility_label)
}

fn detail_value_display_text(value: &str) -> String {
    format!("{value}\u{a0}")
}

fn detail_label_display_text(label: &str) -> String {
    format!("\u{a0}\u{a0}{label}")
}

fn detail_label_frame(base: NSRect, label: &str) -> NSRect {
    if label.chars().count() > 24 {
        NSRect::new(
            NSPoint::new(base.origin.x, base.origin.y - 12.0),
            NSSize::new(base.size.width, base.size.height + 12.0),
        )
    } else {
        base
    }
}

impl StatsTableDataSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(StatsTableDataSourceIvars {
            rows: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn replace_rows(&self, rows: &[ProcessRowViewModel]) -> bool {
        if self.ivars().rows.borrow().as_slice() == rows {
            return false;
        }
        self.ivars().rows.replace(rows.to_vec());
        true
    }

    fn pid_at(&self, row: isize) -> Option<u32> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.ivars().rows.borrow().get(row).map(|row| row.pid))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndicatorFontFallback {
    pub requested_family: FontFamilyPreference,
    pub resolved_family: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndicatorLayoutDiagnostics {
    pub status: Option<LayoutDiagnostics>,
    pub light: LayoutDiagnostics,
    pub dark: LayoutDiagnostics,
}

pub struct IndicatorSurfaceUpdate {
    pub previews: PreviewImages,
    pub font_fallback: Option<IndicatorFontFallback>,
    pub contrast_warnings: PreviewContrastWarnings,
    pub summaries: PreviewSummaries,
    pub layout: IndicatorLayoutDiagnostics,
    pub environment: VisualEnvironment,
}

pub struct PreviewSummaries {
    pub light_visible: String,
    pub dark_visible: String,
    pub light: String,
    pub dark: String,
}

pub struct WindowManager {
    control_target: Retained<ControlTarget>,
    system_usage_generation: Arc<AtomicU64>,
    preferences: Option<PreferencesWindow>,
    history: Option<HistoryWindow>,
    free_space: Option<FreeSpaceWindow>,
    system_usage: Option<SystemUsageWindow>,
}

trait RetainedStateConsumer {
    fn apply_retained_state(&self, state: &AppState);
}

impl RetainedStateConsumer for PreferencesWindow {
    fn apply_retained_state(&self, state: &AppState) {
        self.apply(state, None);
    }
}

impl RetainedStateConsumer for FreeSpaceWindow {
    fn apply_retained_state(&self, state: &AppState) {
        self.apply(state);
    }
}

fn prepare_preferences_for_show<'a, P, F>(
    preferences: &'a mut Option<P>,
    free_space: Option<&F>,
    state: &AppState,
    create: impl FnOnce() -> P,
) -> &'a P
where
    P: RetainedStateConsumer,
    F: RetainedStateConsumer,
{
    get_or_create_window(preferences, create);
    apply_state_to_retained_windows(preferences.as_ref(), free_space, state);
    preferences
        .as_ref()
        .expect("preferences window was created before applying state")
}

fn apply_state_to_retained_windows<P, F>(
    preferences: Option<&P>,
    free_space: Option<&F>,
    state: &AppState,
) where
    P: RetainedStateConsumer,
    F: RetainedStateConsumer,
{
    if let Some(window) = preferences {
        window.apply_retained_state(state);
    }
    if let Some(window) = free_space {
        window.apply_retained_state(state);
    }
}

fn release_preferences<P>(preferences: &mut Option<P>) {
    drop(preferences.take());
}

struct SystemUsageWindow {
    window: Retained<NSWindow>,
    segmented_control: Retained<NSSegmentedControl>,
    primary_value: Retained<NSTextField>,
    secondary_value: Retained<NSTextField>,
    status: Retained<NSTextField>,
    memory_composition: Retained<MemoryCompositionView>,
    graph: Retained<UsageGraphView>,
    history_summary: Retained<NSTextField>,
    detail_labels: Vec<Retained<NSTextField>>,
    detail_values: Vec<Retained<NSTextField>>,
    process_heading: Retained<NSTextField>,
    process_scroll: Retained<NSScrollView>,
    process_table: Retained<NSTableView>,
    process_column: Retained<NSTableColumn>,
    memory_column: Retained<NSTableColumn>,
    process_data_source: Retained<StatsTableDataSource>,
    accessibility: RefCell<SystemUsageAccessibilityCoordinator>,
}

struct SystemUsageLayout {
    segmented: NSRect,
    primary: NSRect,
    secondary: NSRect,
    status: NSRect,
    memory_composition: NSRect,
    detail_labels: [NSRect; 6],
    detail_values: [NSRect; 6],
    history_heading: NSRect,
    graph: NSRect,
    history_summary: NSRect,
    process_heading: NSRect,
    process_scroll: NSRect,
}

const SYSTEM_USAGE_MIN_CONTENT_WIDTH: f64 = 620.0;
const SYSTEM_USAGE_MIN_CONTENT_HEIGHT: f64 = 620.0;
#[cfg(test)]
const PROCESS_TABLE_HEADER_HEIGHT: f64 = 25.0;
const PROCESS_TABLE_ROW_HEIGHT: f64 = 22.0;
const PROCESS_TABLE_ROW_SPACING: f64 = 2.0;

fn system_usage_min_content_size() -> NSSize {
    NSSize::new(
        SYSTEM_USAGE_MIN_CONTENT_WIDTH,
        SYSTEM_USAGE_MIN_CONTENT_HEIGHT,
    )
}

#[cfg(test)]
fn process_table_height_for_visible_rows(rows: usize) -> f64 {
    PROCESS_TABLE_HEADER_HEIGHT
        + rows as f64 * (PROCESS_TABLE_ROW_HEIGHT + PROCESS_TABLE_ROW_SPACING)
}

fn system_usage_layout(size: NSSize) -> SystemUsageLayout {
    let top = size.height;
    let detail_labels = std::array::from_fn(|index| {
        NSRect::new(
            NSPoint::new(size.width - 330.0, top - 100.0 - index as f64 * 30.0),
            NSSize::new(190.0, 24.0),
        )
    });
    let detail_values = std::array::from_fn(|index| {
        NSRect::new(
            NSPoint::new(size.width - 140.0, top - 100.0 - index as f64 * 30.0),
            NSSize::new(116.0, 24.0),
        )
    });
    SystemUsageLayout {
        segmented: NSRect::new(
            NSPoint::new((size.width - 200.0) / 2.0, top - 54.0),
            NSSize::new(200.0, 28.0),
        ),
        primary: NSRect::new(NSPoint::new(24.0, top - 110.0), NSSize::new(240.0, 38.0)),
        secondary: NSRect::new(NSPoint::new(24.0, top - 140.0), NSSize::new(240.0, 24.0)),
        status: NSRect::new(NSPoint::new(24.0, top - 172.0), NSSize::new(240.0, 28.0)),
        memory_composition: NSRect::new(
            NSPoint::new(size.width - 354.0, top - 76.0),
            NSSize::new(330.0, 12.0),
        ),
        detail_labels,
        detail_values,
        history_heading: NSRect::new(NSPoint::new(24.0, top - 250.0), NSSize::new(240.0, 26.0)),
        graph: NSRect::new(
            NSPoint::new(24.0, top - 370.0),
            NSSize::new((size.width - 48.0).max(1.0), 112.0),
        ),
        history_summary: NSRect::new(
            NSPoint::new(24.0, top - 406.0),
            NSSize::new((size.width - 48.0).max(1.0), 30.0),
        ),
        process_heading: NSRect::new(
            NSPoint::new(24.0, top - 438.0),
            NSSize::new((size.width - 48.0).max(1.0), 26.0),
        ),
        process_scroll: NSRect::new(
            NSPoint::new(24.0, 24.0),
            NSSize::new((size.width - 48.0).max(1.0), (size.height - 468.0).max(1.0)),
        ),
    }
}

fn process_table_column_widths(viewport_width: f64) -> (f64, f64) {
    let memory = 130.0_f64.min((viewport_width * 0.28).max(120.0));
    let process = (viewport_width - memory - 20.0).max(240.0);
    (process, memory)
}

fn process_selection_after_update(
    selected_pid: Option<u32>,
    selected_row: isize,
    rows: &[ProcessRowViewModel],
) -> Option<usize> {
    selected_pid
        .and_then(|pid| rows.iter().position(|row| row.pid == pid))
        .or_else(|| {
            usize::try_from(selected_row)
                .ok()
                .filter(|_| !rows.is_empty())
                .map(|row| row.min(rows.len() - 1))
        })
}

fn focused_process_disappearance_announcement(
    selected_pid: Option<u32>,
    interaction_active: bool,
    rows: &[ProcessRowViewModel],
    selection: Option<usize>,
) -> Option<String> {
    let disappeared =
        interaction_active && selected_pid.is_some_and(|pid| rows.iter().all(|row| row.pid != pid));
    disappeared
        .then(|| selection.and_then(|index| rows.get(index)))
        .flatten()
        .map(|row| {
            format!(
                "O processo selecionado terminou; seleção movida para {}.",
                row.name
            )
        })
}

fn accessibility_object_is_inside(mut object: Retained<AnyObject>, ancestor: &AnyObject) -> bool {
    for _ in 0..16 {
        let matches: bool = unsafe { msg_send![&*object, isEqual: ancestor] };
        if matches {
            return true;
        }
        let parent: Option<Retained<AnyObject>> =
            unsafe { msg_send![&*object, accessibilityParent] };
        let Some(parent) = parent else {
            return false;
        };
        object = parent;
    }
    false
}

impl WindowManager {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let system_usage_visible = Arc::new(AtomicBool::new(false));
        let system_usage_generation = Arc::new(AtomicU64::new(0));
        let control_target = ControlTarget::new(
            mtm,
            proxy,
            system_usage_visible,
            Arc::clone(&system_usage_generation),
        );
        Self {
            control_target,
            system_usage_generation,
            preferences: None,
            history: None,
            free_space: None,
            system_usage: None,
        }
    }

    pub fn show(&mut self, kind: WindowKind, state: &AppState, history: &History) {
        let shows_system_usage = kind == WindowKind::SystemUsage;
        let mtm = MainThreadMarker::new().expect("native window actions run on the main thread");
        let window = match kind {
            WindowKind::Preferences => {
                let target = self.control_target.clone();
                let preferences = prepare_preferences_for_show(
                    &mut self.preferences,
                    self.free_space.as_ref(),
                    state,
                    || PreferencesWindow::new(mtm, &target),
                );
                &preferences.window
            }
            WindowKind::History => {
                if self.history.is_none() {
                    self.history = Some(create_history_window(mtm, &self.control_target));
                }
                self.update_history(history);
                &self
                    .history
                    .as_ref()
                    .expect("history window was created")
                    .window
            }
            WindowKind::FreeSpace => {
                if self.free_space.is_none() {
                    self.free_space = Some(create_free_space_window(mtm, &self.control_target));
                }
                self.update_state(state);
                &self
                    .free_space
                    .as_ref()
                    .expect("free-space window was created")
                    .window
            }
            WindowKind::SystemUsage => {
                if self.system_usage.is_none() {
                    self.system_usage = Some(create_system_usage_window(mtm, &self.control_target));
                }
                &self
                    .system_usage
                    .as_ref()
                    .expect("system-usage window was created")
                    .window
            }
        };

        let app = NSApplication::sharedApplication(mtm);
        // A window is shown only after an explicit launch, menu choice, or
        // notification click, so request cooperative activation before
        // promoting the retained window.
        app.activate();
        if shows_system_usage {
            self.control_target.update_system_usage_visibility(true);
        }
        window.makeKeyAndOrderFront(None);
    }

    pub fn update_state(&self, state: &AppState) {
        apply_state_to_retained_windows(self.preferences.as_ref(), self.free_space.as_ref(), state);
    }

    pub fn update_history(&self, history: &History) {
        if let Some(window) = &self.history {
            window.apply(history);
        }
    }
    pub fn system_usage_visible(&self) -> bool {
        self.system_usage
            .as_ref()
            .is_some_and(|window| window.window.isVisible() && !window.window.isMiniaturized())
    }

    pub fn system_usage_visibility_generation(&self) -> u64 {
        self.system_usage_generation.load(Ordering::Acquire)
    }

    pub fn system_usage_process_interaction_active(&self) -> bool {
        self.system_usage
            .as_ref()
            .is_some_and(SystemUsageWindow::process_interaction_active)
    }

    pub fn system_usage_observation(&self) -> SurfaceObservation {
        SurfaceObservation {
            visible: self.system_usage_visible(),
            native_visibility_epoch: self.system_usage_visibility_generation(),
            process_interaction_active: self.system_usage_process_interaction_active(),
        }
    }

    pub fn request_system_usage_summary_focus(&self, section: SystemUsageSection) {
        if let Some(window) = &self.system_usage {
            window
                .accessibility
                .borrow_mut()
                .request_summary_focus_after_user_switch(section);
        }
    }

    pub fn update_system_usage(&self, view_model: &SystemUsageViewModel) {
        if let Some(window) = &self.system_usage {
            window.apply(view_model);
        }
    }

    pub fn has_preferences_surface(&self) -> bool {
        self.preferences
            .as_ref()
            .is_some_and(PreferencesWindow::is_created_and_visible)
    }

    pub fn update_indicator_surfaces(&self, surfaces: IndicatorSurfaceUpdate) {
        if let Some(window) = &self.preferences {
            window.apply_surfaces(surfaces);
        }
    }

    pub fn release_preferences(&mut self) {
        release_preferences(&mut self.preferences);
    }
}

impl SystemUsageWindow {
    fn process_interaction_active(&self) -> bool {
        let keyboard_focused = self
            .window
            .firstResponder()
            .is_some_and(|responder| unsafe {
                msg_send![&*responder, isEqual: &*self.process_table]
            });
        if keyboard_focused {
            return true;
        }
        let mtm = MainThreadMarker::new().expect("focus is inspected on the main thread");
        NSApplication::sharedApplication(mtm)
            .accessibilityApplicationFocusedUIElement()
            .is_some_and(|focused| accessibility_object_is_inside(focused, &self.process_table))
    }

    fn apply(&self, view_model: &SystemUsageViewModel) {
        let layout = system_usage_layout(
            self.window
                .contentView()
                .expect("system-usage content view")
                .bounds()
                .size,
        );
        let mut accessibility_update = self
            .accessibility
            .borrow_mut()
            .observe(view_model.section, view_model.accessibility_state);
        let process_viewport_width = self.process_scroll.contentView().bounds().size.width;
        let (process_width, memory_width) = process_table_column_widths(process_viewport_width);
        self.process_table.setFrameSize(NSSize::new(
            process_viewport_width,
            self.process_table.frame().size.height,
        ));
        self.process_column.setWidth(process_width);
        self.memory_column.setWidth(memory_width);
        self.segmented_control
            .setSelectedSegment(match view_model.section {
                SystemUsageSection::Ram => 0,
                SystemUsageSection::Gpu => 1,
            });
        self.primary_value
            .setStringValue(&NSString::from_str(&view_model.primary_value));
        self.secondary_value
            .setStringValue(&NSString::from_str(&view_model.secondary_value));
        self.secondary_value
            .setHidden(view_model.secondary_value.is_empty());
        self.status
            .setStringValue(&NSString::from_str(&view_model.status));
        self.memory_composition.apply(
            &view_model.memory_composition,
            &view_model.memory_composition_accessibility_label,
        );
        self.memory_composition.setHidden(
            view_model.section != SystemUsageSection::Ram
                || view_model.memory_composition.is_empty(),
        );
        self.primary_value
            .setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{}, {}, {}",
                view_model.primary_value, view_model.secondary_value, view_model.status
            ))));
        self.graph
            .apply(&view_model.history, &view_model.history_accessibility_label);
        self.history_summary
            .setStringValue(&NSString::from_str(&view_model.history_accessibility_label));

        for (index, label) in self.detail_labels.iter().enumerate() {
            let Some(detail) = view_model.details.get(index) else {
                label.setHidden(true);
                self.detail_values[index].setHidden(true);
                continue;
            };
            label.setFrame(detail_label_frame(
                layout.detail_labels[index],
                detail.label,
            ));
            label.setMaximumNumberOfLines(if detail.label.chars().count() > 24 {
                2
            } else {
                1
            });
            label.setUsesSingleLineMode(detail.label.chars().count() <= 24);
            label.setLineBreakMode(if detail.label.chars().count() > 24 {
                NSLineBreakMode::ByWordWrapping
            } else {
                NSLineBreakMode::ByClipping
            });
            self.detail_values[index].setFrame(layout.detail_values[index]);
            label.setHidden(false);
            label.setStringValue(&NSString::from_str(&detail_label_display_text(
                detail.label,
            )));
            label.setAccessibilityLabel(Some(&NSString::from_str(detail.label)));
            self.detail_values[index].setHidden(false);
            self.detail_values[index].setStringValue(&NSString::from_str(
                &detail_value_display_text(&detail.value),
            ));
            self.detail_values[index].setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{}: {}",
                detail.label, detail.value
            ))));
        }

        let shows_processes = view_model.section == SystemUsageSection::Ram;
        self.process_heading.setHidden(!shows_processes);
        let process_heading = match view_model.process_status {
            ProcessListStatus::Available => "Maiores usos de memória agora".to_owned(),
            ProcessListStatus::Stale => {
                "Maiores usos de memória agora — dados desatualizados".to_owned()
            }
            status => status.message().to_owned(),
        };
        self.process_heading
            .setStringValue(&NSString::from_str(&process_heading));
        self.process_heading
            .setAccessibilityLabel(Some(&NSString::from_str(&process_heading)));
        let shows_process_rows = matches!(
            view_model.process_status,
            ProcessListStatus::Available | ProcessListStatus::Stale
        );
        self.process_scroll
            .setHidden(!shows_processes || !shows_process_rows);
        let selected_row = self.process_table.selectedRow();
        let selected_pid = self.process_data_source.pid_at(selected_row);
        let process_interaction_active = self.process_interaction_active();
        let selection =
            process_selection_after_update(selected_pid, selected_row, &view_model.process_rows);
        if let Some(announcement) = focused_process_disappearance_announcement(
            selected_pid,
            process_interaction_active,
            &view_model.process_rows,
            selection,
        ) {
            accessibility_update.include_announcement(announcement);
        }
        if self
            .process_data_source
            .replace_rows(&view_model.process_rows)
        {
            let clip_view = self.process_scroll.contentView();
            let scroll_origin = clip_view.bounds().origin;
            self.process_table.reloadData();
            clip_view.scrollToPoint(scroll_origin);
            self.process_scroll.reflectScrolledClipView(&clip_view);
            if let Some(index) = selection {
                self.process_table.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(index),
                    false,
                );
            }
        }
        if accessibility_update.focus_summary {
            self.primary_value.setAccessibilityFocused(true);
        }
        if let Some(announcement) = accessibility_update.announcement {
            post_accessibility_announcement(&self.primary_value, &announcement);
        }
    }
}

fn post_accessibility_announcement(element: &AnyObject, announcement: &str) {
    let user_info = accessibility_announcement_user_info(announcement);
    unsafe {
        NSAccessibilityPostNotificationWithUserInfo(
            element,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&user_info),
        )
    };
}

fn accessibility_announcement_user_info(
    announcement: &str,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    NSDictionary::from_retained_objects(
        &[unsafe { NSAccessibilityAnnouncementKey }, unsafe {
            NSAccessibilityPriorityKey
        }],
        &[
            Retained::into_super(Retained::into_super(NSString::from_str(announcement))),
            Retained::into_super(Retained::into_super(Retained::into_super(
                NSNumber::numberWithInteger(NSAccessibilityPriorityLevel::Medium.0),
            ))),
        ],
    )
}

fn create_system_usage_window(mtm: MainThreadMarker, target: &ControlTarget) -> SystemUsageWindow {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(680.0, 620.0)),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setAcceptsMouseMovedEvents(true);
    window.setDelegate(Some(ProtocolObject::from_ref(target)));
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);
    window.setTitle(ns_string!("Uso do sistema"));
    window.setContentMinSize(system_usage_min_content_size());
    window.center();
    let content = window
        .contentView()
        .expect("system-usage window content view");
    let layout = system_usage_layout(content.bounds().size);
    let top_left_mask =
        NSAutoresizingMaskOptions::ViewMaxXMargin | NSAutoresizingMaskOptions::ViewMinYMargin;
    let top_center_mask = NSAutoresizingMaskOptions::ViewMinXMargin
        | NSAutoresizingMaskOptions::ViewMaxXMargin
        | NSAutoresizingMaskOptions::ViewMinYMargin;
    let top_right_mask =
        NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin;
    let full_width_top_mask =
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin;

    let labels =
        NSArray::from_retained_slice(&[NSString::from_str("RAM"), NSString::from_str("GPU")]);
    let segmented_control = unsafe {
        NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
            &labels,
            NSSegmentSwitchTracking::SelectOne,
            Some(target as &AnyObject),
            Some(sel!(changeSystemUsageSection:)),
            mtm,
        )
    };
    segmented_control.setFrame(layout.segmented);
    segmented_control.setAutoresizingMask(top_center_mask);
    segmented_control.setSelectedSegment(0);
    segmented_control.setAccessibilityLabel(Some(ns_string!("Seção de uso do sistema")));

    let primary_value = text_label(mtm, "—", layout.primary);
    primary_value.setAutoresizingMask(top_left_mask);
    primary_value.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(28.0)));
    let secondary_value = text_label(mtm, "", layout.secondary);
    secondary_value.setAutoresizingMask(top_left_mask);
    let status = text_label(mtm, "Coletando a primeira leitura…", layout.status);
    status.setAutoresizingMask(top_left_mask);
    status.setTextColor(Some(&NSColor::secondaryLabelColor()));
    let memory_composition = MemoryCompositionView::new(mtm, layout.memory_composition);
    memory_composition.setAutoresizingMask(top_right_mask);
    memory_composition.setHidden(true);

    let mut detail_labels = Vec::with_capacity(6);
    let mut detail_values = Vec::with_capacity(6);
    for index in 0..6 {
        let label = text_label(mtm, "", layout.detail_labels[index]);
        label.setAutoresizingMask(top_right_mask);
        let value = text_label(mtm, "", layout.detail_values[index]);
        value.setAutoresizingMask(top_right_mask);
        value.setAlignment(objc2_app_kit::NSTextAlignment::Right);
        value.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(13.0)));
        label.setHidden(true);
        value.setHidden(true);
        content.addSubview(&label);
        content.addSubview(&value);
        detail_labels.push(label);
        detail_values.push(value);
    }

    let history_heading = text_label(mtm, "Últimos 5 minutos", layout.history_heading);
    history_heading.setAutoresizingMask(top_left_mask);
    history_heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(15.0)));
    let graph = UsageGraphView::new(mtm, layout.graph);
    graph.setAutoresizingMask(full_width_top_mask);
    let history_summary = text_label(
        mtm,
        "O histórico aparecerá após duas leituras.",
        layout.history_summary,
    );
    history_summary.setTextColor(Some(&NSColor::secondaryLabelColor()));
    history_summary.setAutoresizingMask(full_width_top_mask);

    let process_heading = text_label(mtm, "Maiores usos de memória agora", layout.process_heading);
    process_heading.setAutoresizingMask(full_width_top_mask);
    process_heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(15.0)));
    let process_scroll =
        NSScrollView::initWithFrame(NSScrollView::alloc(mtm), layout.process_scroll);
    process_scroll.setHasVerticalScroller(true);
    process_scroll.setDrawsBackground(false);
    process_scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let process_table = NSTableView::initWithFrame(
        NSTableView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new((layout.process_scroll.size.width - 20.0).max(1.0), 188.0),
        ),
    );
    process_table.setRowHeight(PROCESS_TABLE_ROW_HEIGHT);
    process_table.setIntercellSpacing(NSSize::new(3.0, PROCESS_TABLE_ROW_SPACING));
    process_table.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    process_table.setColumnAutoresizingStyle(
        NSTableViewColumnAutoresizingStyle::FirstColumnOnlyAutoresizingStyle,
    );
    let (process_width, memory_width) =
        process_table_column_widths((layout.process_scroll.size.width - 20.0).max(1.0));
    let process_column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("process"));
    process_column.setWidth(process_width);
    process_column.setMinWidth(240.0);
    process_column.setResizingMask(
        NSTableColumnResizingOptions::AutoresizingMask
            | NSTableColumnResizingOptions::UserResizingMask,
    );
    process_column
        .headerCell()
        .setStringValue(ns_string!("Processo"));
    let memory_column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("memory"));
    memory_column.setWidth(memory_width);
    memory_column.setMinWidth(120.0);
    memory_column.setMaxWidth(160.0);
    memory_column.setResizingMask(NSTableColumnResizingOptions::UserResizingMask);
    memory_column
        .headerCell()
        .setStringValue(ns_string!("Memória"));
    process_table.addTableColumn(&process_column);
    process_table.addTableColumn(&memory_column);
    process_table.setAccessibilityLabel(Some(ns_string!(
        "Maiores usos de memória agora, colunas Processo e Memória"
    )));
    let process_data_source = StatsTableDataSource::new(mtm);
    unsafe {
        process_table.setDataSource(Some(ProtocolObject::from_ref(&*process_data_source)));
        process_table.setDelegate(Some(ProtocolObject::from_ref(&*process_data_source)));
    }
    process_scroll.setDocumentView(Some(&process_table));

    let [ram_key, gpu_key, close_key] = system_usage_shortcut_keys();
    let ram_shortcut = shortcut_button(
        mtm,
        target as &AnyObject,
        ram_key,
        sel!(selectSystemUsageRam:),
    );
    let gpu_shortcut = shortcut_button(
        mtm,
        target as &AnyObject,
        gpu_key,
        sel!(selectSystemUsageGpu:),
    );
    let close_shortcut =
        shortcut_button(mtm, &*window as &AnyObject, close_key, sel!(performClose:));

    content.addSubview(&segmented_control);
    content.addSubview(&primary_value);
    content.addSubview(&secondary_value);
    content.addSubview(&status);
    content.addSubview(&memory_composition);
    content.addSubview(&history_heading);
    content.addSubview(&graph);
    content.addSubview(&history_summary);
    content.addSubview(&process_heading);
    content.addSubview(&process_scroll);
    content.addSubview(&ram_shortcut);
    content.addSubview(&gpu_shortcut);
    content.addSubview(&close_shortcut);
    window.setInitialFirstResponder(Some(&segmented_control));

    SystemUsageWindow {
        window,
        segmented_control,
        primary_value,
        secondary_value,
        status,
        memory_composition,
        graph,
        history_summary,
        detail_labels,
        detail_values,
        process_heading,
        process_scroll,
        process_table,
        process_column,
        memory_column,
        process_data_source,
        accessibility: RefCell::new(SystemUsageAccessibilityCoordinator::new()),
    }
}

fn shortcut_button(
    mtm: MainThreadMarker,
    target: &AnyObject,
    key: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(ns_string!(""), Some(target), Some(action), mtm)
    };
    button.setFrame(NSRect::new(
        NSPoint::new(-20.0, -20.0),
        NSSize::new(1.0, 1.0),
    ));
    button.setKeyEquivalent(&NSString::from_str(key));
    button.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
    button.setAccessibilityElement(false);
    button
}

fn system_usage_shortcut_keys() -> [&'static str; 3] {
    ["1", "2", "w"]
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use statlet::core::{AppState, StatletCore};

    use super::{prepare_preferences_for_show, release_preferences, RetainedStateConsumer};

    #[test]
    fn system_usage_shortcuts_include_the_native_close_command() {
        assert_eq!(super::system_usage_shortcut_keys(), ["1", "2", "w"]);
    }

    #[derive(Default)]
    struct RecordingStateConsumer {
        applications: RefCell<Vec<AppState>>,
    }

    impl RetainedStateConsumer for RecordingStateConsumer {
        fn apply_retained_state(&self, state: &AppState) {
            self.applications.borrow_mut().push(state.clone());
        }
    }

    #[test]
    fn opening_preferences_refreshes_a_retained_free_space_window() {
        let state = StatletCore::new().state().clone();
        let mut preferences = None;
        let free_space = RecordingStateConsumer::default();

        let preferences = prepare_preferences_for_show(
            &mut preferences,
            Some(&free_space),
            &state,
            RecordingStateConsumer::default,
        );

        assert_eq!(
            preferences.applications.borrow().as_slice(),
            std::slice::from_ref(&state)
        );
        assert_eq!(free_space.applications.borrow().len(), 1);
        assert_eq!(
            free_space.applications.borrow().as_slice(),
            std::slice::from_ref(&state)
        );
    }

    #[test]
    fn closing_then_reopening_preferences_rebuilds_from_the_latest_app_state() {
        let mut preferences = Some(RecordingStateConsumer::default());
        release_preferences(&mut preferences);
        assert!(preferences.is_none());

        let mut core = StatletCore::new();
        core.handle(statlet::core::AppEvent::SetMoleIntegrationEnabled(true));
        let latest = core.state().clone();
        let reopened = prepare_preferences_for_show(
            &mut preferences,
            None::<&RecordingStateConsumer>,
            &latest,
            RecordingStateConsumer::default,
        );

        assert_eq!(
            reopened.applications.borrow().as_slice(),
            std::slice::from_ref(&latest)
        );
    }
}

#[cfg(test)]
mod system_usage_tests {
    use super::{
        accessibility_announcement_user_info, detail_label_display_text, detail_label_frame,
        detail_value_display_text, focused_process_disappearance_announcement,
        memory_composition_stroke_style, process_cell_presentation, process_selection_after_update,
        process_table_column_widths, process_table_height_for_visible_rows, system_usage_layout,
        system_usage_min_content_size,
    };
    use objc2::msg_send;
    use objc2_app_kit::{
        NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey, NSAccessibilityPriorityLevel,
    };
    use objc2_foundation::NSSize;
    use statlet::system_usage::ProcessRowViewModel;

    #[test]
    fn native_announcement_payload_includes_message_and_medium_priority() {
        let payload = accessibility_announcement_user_info("Pressão da memória crítica.");

        assert_eq!(payload.count(), 2);
        let announcement = payload
            .objectForKey(unsafe { NSAccessibilityAnnouncementKey })
            .expect("announcement value");
        let priority = payload
            .objectForKey(unsafe { NSAccessibilityPriorityKey })
            .expect("priority value");
        let announcement: String = unsafe {
            let description: objc2::rc::Retained<objc2_foundation::NSString> =
                msg_send![&*announcement, description];
            description.to_string()
        };
        let priority: isize = unsafe { msg_send![&*priority, integerValue] };

        assert_eq!(announcement, "Pressão da memória crítica.");
        assert_eq!(priority, NSAccessibilityPriorityLevel::Medium.0);
    }

    #[test]
    fn native_process_cells_have_distinct_column_aware_accessibility_labels() {
        let row = ProcessRowViewModel {
            pid: 42,
            name: "Safari".to_owned(),
            memory: "512 MB".to_owned(),
        };

        assert_eq!(
            process_cell_presentation(&row, "process"),
            ("Safari".to_owned(), "Processo: Safari".to_owned())
        );
        assert_eq!(
            process_cell_presentation(&row, "memory"),
            ("512 MB".to_owned(), "Memória: 512 MB".to_owned())
        );
    }

    #[test]
    fn gpu_long_detail_wraps_without_overlapping_history_and_values_keep_right_padding() {
        let layout = system_usage_layout(NSSize::new(620.0, 520.0));
        let base = layout.detail_labels[3];
        let wrapped = detail_label_frame(base, "Memória compartilhada em uso");

        assert_eq!(wrapped.size.height, 36.0);
        assert_eq!(
            wrapped.origin.y + wrapped.size.height,
            base.origin.y + base.size.height
        );
        assert!(wrapped.origin.y > layout.graph.origin.y + layout.graph.size.height);
        assert_eq!(detail_value_display_text("937,1 MB"), "937,1 MB\u{a0}");
        assert_eq!(
            detail_label_display_text("Renderer"),
            "\u{a0}\u{a0}Renderer"
        );
    }

    #[test]
    fn memory_composition_outline_and_dividers_strengthen_for_increase_contrast() {
        let normal = memory_composition_stroke_style(false);
        let increased = memory_composition_stroke_style(true);

        assert_eq!(normal.outline_width, 1.0);
        assert_eq!(normal.divider_width, 1.0);
        assert_eq!(increased.outline_width, 3.0);
        assert_eq!(increased.divider_width, 2.0);
    }

    #[test]
    fn system_usage_layout_fits_without_overlap_at_the_declared_minimum() {
        let minimum = system_usage_min_content_size();
        let layout = system_usage_layout(minimum);
        let max_x = |frame: objc2_foundation::NSRect| frame.origin.x + frame.size.width;
        let max_y = |frame: objc2_foundation::NSRect| frame.origin.y + frame.size.height;

        assert_eq!(minimum, NSSize::new(620.0, 620.0));
        assert!(max_x(layout.segmented) <= minimum.width);
        assert!(max_y(layout.segmented) <= minimum.height);
        assert!(max_x(layout.primary) < layout.detail_labels[0].origin.x);
        assert!(max_x(layout.memory_composition) <= minimum.width);
        assert!(max_y(layout.detail_labels[0]) <= layout.memory_composition.origin.y);
        assert!(max_y(layout.memory_composition) < layout.segmented.origin.y);
        assert!(max_x(layout.history_heading) < layout.detail_labels[4].origin.x);
        assert!(max_y(layout.graph) < layout.detail_values[5].origin.y);
        assert!(max_y(layout.history_summary) < layout.graph.origin.y);
        assert!(max_y(layout.process_heading) < layout.history_summary.origin.y);
        assert!(max_y(layout.process_scroll) < layout.process_heading.origin.y);
        assert!(
            layout.process_scroll.size.height >= process_table_height_for_visible_rows(5),
            "the declared minimum must show the header plus five process rows"
        );
    }

    #[test]
    fn process_table_columns_fit_the_minimum_viewport_without_hiding_memory() {
        let (process, memory) = process_table_column_widths(552.0);

        assert!(process + memory + 20.0 <= 552.0);
        assert!(process >= 400.0);
        assert!(memory >= 120.0);
    }

    #[test]
    fn process_selection_is_preserved_by_pid_or_moves_to_the_nearest_row() {
        let rows = [
            ProcessRowViewModel {
                pid: 7,
                name: "A".to_owned(),
                memory: "2 GB".to_owned(),
            },
            ProcessRowViewModel {
                pid: 42,
                name: "B".to_owned(),
                memory: "1 GB".to_owned(),
            },
        ];

        assert_eq!(process_selection_after_update(Some(42), 0, &rows), Some(1));
        assert_eq!(process_selection_after_update(Some(99), 8, &rows), Some(1));
        assert_eq!(process_selection_after_update(None, -1, &rows), None);
    }

    #[test]
    fn focused_process_disappearance_announces_the_new_selection_only_during_interaction() {
        let rows = [ProcessRowViewModel {
            pid: 7,
            name: "Orca".to_owned(),
            memory: "2 GB".to_owned(),
        }];

        assert_eq!(
            focused_process_disappearance_announcement(Some(42), true, &rows, Some(0)),
            Some("O processo selecionado terminou; seleção movida para Orca.".to_owned())
        );
        assert_eq!(
            focused_process_disappearance_announcement(Some(42), false, &rows, Some(0)),
            None
        );
        assert_eq!(
            focused_process_disappearance_announcement(Some(7), true, &rows, Some(0)),
            None
        );
    }
}
