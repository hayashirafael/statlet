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
    NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityValueChangedNotification, NSAlert,
    NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezierPath, NSButton, NSColor, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSEvent, NSEventModifierFlags, NSFocusRingType, NSLineBreakMode,
    NSPopUpButton, NSScrollView, NSSegmentSwitchTracking, NSSegmentedControl, NSTableColumn,
    NSTableColumnResizingOptions, NSTableView, NSTableViewColumnAutoresizingStyle,
    NSTableViewDataSource, NSTableViewDelegate, NSTextField, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSWindow, NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSDate, NSDateFormatter, NSDateFormatterStyle,
    NSDictionary, NSFileManager, NSIndexSet, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString,
};
use statlet::core::{AppEvent, AppState, Preferences, WarningThreshold, WindowKind};
use statlet::disk::format_decimal_gigabytes;
use statlet::history::{History, HistoryEventKind, HistoryRecord, MAX_HISTORY_RECORDS};
use statlet::mole::MoleStatus;
use statlet::stats::{
    graph_pointer_selection, history_x_position, GraphNavigation, GraphNavigationCommand,
    MemoryCompositionSegment, ProcessListStatus, ProcessRowViewModel,
    SystemUsageAccessibilityCoordinator, SystemUsageSection, SystemUsageViewModel, UsagePoint,
};
use tao::event_loop::EventLoopProxy;

use super::RuntimeEvent;

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

struct ControlTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    system_usage_visible: Arc<AtomicBool>,
    system_usage_generation: Arc<AtomicU64>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControlTargetIvars]
    struct ControlTarget;

    unsafe impl NSObjectProtocol for ControlTarget {}

    unsafe impl NSWindowDelegate for ControlTarget {
        #[unsafe(method(windowDidBecomeKey:))]
        fn system_usage_became_key(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(true);
        }

        #[unsafe(method(windowWillClose:))]
        fn system_usage_will_close(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(false);
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn system_usage_did_miniaturize(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(false);
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn system_usage_did_deminiaturize(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(true);
        }
    }

    impl ControlTarget {
        #[unsafe(method(toggleMoleIntegration:))]
        fn toggle_mole_integration(&self, sender: &NSButton) {
            let enabled = sender.state() == NSControlStateValueOn;
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::SetMoleIntegrationEnabled(
                    enabled,
                )));
        }

        #[unsafe(method(changeWarningThreshold:))]
        fn change_warning_threshold(&self, sender: &NSPopUpButton) {
            let Some(title) = sender.titleOfSelectedItem() else {
                return;
            };
            let Ok(value) = title.to_string().trim_end_matches('%').parse::<u8>() else {
                return;
            };
            let Ok(threshold) = WarningThreshold::try_from(value) else {
                return;
            };
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::SetWarningThreshold(threshold)));
        }

        #[unsafe(method(changeSystemUsageSection:))]
        fn change_system_usage_section(&self, sender: &NSSegmentedControl) {
            let section = if sender.selectedSegment() == 1 {
                SystemUsageSection::Gpu
            } else {
                SystemUsageSection::Ram
            };
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::SystemUsageSectionSelectedByUser(section));
        }

        #[unsafe(method(selectSystemUsageRam:))]
        fn select_system_usage_ram(&self, _sender: &NSButton) {
            let _ = self.ivars().proxy.send_event(
                RuntimeEvent::SystemUsageSectionSelectedByUser(SystemUsageSection::Ram),
            );
        }

        #[unsafe(method(selectSystemUsageGpu:))]
        fn select_system_usage_gpu(&self, _sender: &NSButton) {
            let _ = self.ivars().proxy.send_event(
                RuntimeEvent::SystemUsageSectionSelectedByUser(SystemUsageSection::Gpu),
            );
        }

        #[unsafe(method(openMoleInTerminal:))]
        fn open_mole_in_terminal(&self, _sender: &NSButton) {
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::OpenMoleInTerminal));
        }

        #[unsafe(method(clearHistory:))]
        fn clear_history(&self, _sender: &NSButton) {
            let mtm = MainThreadMarker::new().expect("history actions run on the main thread");
            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Warning);
            alert.setMessageText(ns_string!("Apagar todo o histórico?"));
            alert.setInformativeText(ns_string!(
                "Esta ação remove os registros locais do Statlet e não pode ser desfeita."
            ));
            alert.addButtonWithTitle(ns_string!("Apagar histórico"));
            alert.addButtonWithTitle(ns_string!("Cancelar"));
            if alert.runModal() == NSAlertFirstButtonReturn {
                let _ = self
                    .ivars()
                    .proxy
                    .send_event(RuntimeEvent::App(AppEvent::ClearHistoryConfirmed));
            }
        }
    }
);

impl ControlTarget {
    fn new(
        mtm: MainThreadMarker,
        proxy: EventLoopProxy<RuntimeEvent>,
        system_usage_visible: Arc<AtomicBool>,
        system_usage_generation: Arc<AtomicU64>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ControlTargetIvars {
            proxy,
            system_usage_visible,
            system_usage_generation,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn update_system_usage_visibility(&self, visible: bool) {
        let changed = self
            .ivars()
            .system_usage_visible
            .swap(visible, Ordering::AcqRel)
            != visible;
        if changed {
            self.ivars()
                .system_usage_generation
                .fetch_add(1, Ordering::AcqRel);
        }
        let _ = self
            .ivars()
            .proxy
            .send_event(RuntimeEvent::SystemUsageVisibilityChanged(visible));
    }
}

pub struct WindowManager {
    control_target: Retained<ControlTarget>,
    system_usage_generation: Arc<AtomicU64>,
    preferences: Option<PreferencesWindow>,
    history: Option<HistoryWindow>,
    free_space: Option<FreeSpaceWindow>,
    system_usage: Option<SystemUsageWindow>,
}

struct PreferencesWindow {
    window: Retained<NSWindow>,
    mole_checkbox: Retained<NSButton>,
    warning_threshold: Retained<NSPopUpButton>,
}

struct FreeSpaceWindow {
    window: Retained<NSWindow>,
    occupied_value: Retained<NSTextField>,
    available_value: Retained<NSTextField>,
    threshold_value: Retained<NSTextField>,
    mole_status: Retained<NSTextField>,
    open_mole_button: Retained<NSButton>,
}

struct HistoryWindow {
    window: Retained<NSWindow>,
    document: Retained<NSView>,
    rows: Vec<Retained<NSTextField>>,
    empty_label: Retained<NSTextField>,
    scroll_view: Retained<NSScrollView>,
    clear_button: Retained<NSButton>,
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
                if self.preferences.is_none() {
                    self.preferences = Some(create_preferences_window(mtm, &self.control_target));
                }
                self.update_state(state);
                &self
                    .preferences
                    .as_ref()
                    .expect("preferences window was created")
                    .window
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
        if let Some(window) = &self.preferences {
            window.apply(state.preferences);
        }
        if let Some(window) = &self.free_space {
            window.apply(state);
        }
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

impl FreeSpaceWindow {
    fn apply(&self, state: &AppState) {
        let (occupied, available) = state
            .latest_disk_observation
            .map(|observation| {
                (
                    format!("{:.1}%", observation.occupied_percent()),
                    format_decimal_gigabytes(observation.available_bytes()),
                )
            })
            .unwrap_or_else(|| ("Aguardando leitura".to_owned(), "—".to_owned()));
        self.occupied_value
            .setStringValue(&objc2_foundation::NSString::from_str(&occupied));
        self.occupied_value
            .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
                "Ocupado: {occupied}"
            ))));
        self.available_value
            .setStringValue(&objc2_foundation::NSString::from_str(&available));
        self.available_value
            .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
                "Disponível para uso importante: {available}"
            ))));
        let threshold = format!("{}%", state.preferences.warning_threshold.get());
        self.threshold_value
            .setStringValue(&objc2_foundation::NSString::from_str(&threshold));
        self.threshold_value
            .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
                "Limite configurado: {threshold}"
            ))));

        let (status, enabled) = match state.mole_status {
            MoleStatus::Unknown => ("Verificando a instalação do Mole…".to_owned(), false),
            MoleStatus::Compatible(version) => (
                format!(
                    "Mole {}.{}.{} pronto para abrir no Terminal.",
                    version.major, version.minor, version.patch
                ),
                true,
            ),
            MoleStatus::Missing => (
                "Mole não encontrado. Instale-o pelo site oficial e tente novamente.".to_owned(),
                false,
            ),
            MoleStatus::Unavailable => (
                "Não foi possível validar o Mole. Atualize ou reinstale e tente novamente."
                    .to_owned(),
                false,
            ),
            MoleStatus::Incompatible(version) => (
                format!(
                    "Mole {}.{}.{} não é compatível. Atualize para uma versão 1.x recente.",
                    version.major, version.minor, version.patch
                ),
                false,
            ),
        };
        self.mole_status
            .setStringValue(&objc2_foundation::NSString::from_str(&status));
        self.mole_status
            .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
                "Estado da integração do Mole: {status}"
            ))));
        self.open_mole_button.setEnabled(enabled);
    }
}

impl PreferencesWindow {
    fn apply(&self, preferences: Preferences) {
        self.mole_checkbox
            .setState(if preferences.mole_integration_enabled {
                NSControlStateValueOn
            } else {
                0
            });
        self.warning_threshold
            .selectItemWithTitle(&threshold_title(preferences.warning_threshold));
        self.warning_threshold
            .setEnabled(preferences.mole_integration_enabled);
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
    let user_info = NSDictionary::from_retained_objects(
        &[unsafe { NSAccessibilityAnnouncementKey }],
        &[Retained::into_super(Retained::into_super(
            NSString::from_str(announcement),
        ))],
    );
    unsafe {
        NSAccessibilityPostNotificationWithUserInfo(
            element,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&user_info),
        )
    };
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
    window.setContentMinSize(NSSize::new(620.0, 520.0));
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

    let ram_shortcut = shortcut_button(mtm, target, "1", sel!(selectSystemUsageRam:));
    let gpu_shortcut = shortcut_button(mtm, target, "2", sel!(selectSystemUsageGpu:));

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
    target: &ControlTarget,
    key: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!(""),
            Some(target as &AnyObject),
            Some(action),
            mtm,
        )
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

fn create_preferences_window(mtm: MainThreadMarker, target: &ControlTarget) -> PreferencesWindow {
    let window = create_window(mtm, "Preferências do Statlet", NSSize::new(480.0, 238.0));
    let content = window
        .contentView()
        .expect("preferences window content view");

    let heading = NSTextField::labelWithString(ns_string!("Disco e Mole"), mtm);
    heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(15.0)));
    heading.setFrame(NSRect::new(
        NSPoint::new(24.0, 184.0),
        NSSize::new(420.0, 24.0),
    ));

    let checkbox = unsafe {
        NSButton::checkboxWithTitle_target_action(
            ns_string!("Monitorar o disco com a integração do Mole"),
            Some(target as &AnyObject),
            Some(sel!(toggleMoleIntegration:)),
            mtm,
        )
    };
    checkbox.setFrame(NSRect::new(
        NSPoint::new(24.0, 144.0),
        NSSize::new(410.0, 24.0),
    ));
    checkbox.setAccessibilityLabel(Some(ns_string!(
        "Monitorar o disco com a integração do Mole"
    )));
    checkbox.setAccessibilityHelp(Some(ns_string!(
        "Ativa os avisos de pouco espaço e mostra o badge de disco no indicador."
    )));

    let explanation = NSTextField::labelWithString(
        ns_string!(
            "O Statlet apenas avisa quando o limite for mantido.\nA limpeza é feita fora do app."
        ),
        mtm,
    );
    explanation.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    explanation.setMaximumNumberOfLines(2);
    explanation.setFrame(NSRect::new(
        NSPoint::new(44.0, 102.0),
        NSSize::new(410.0, 38.0),
    ));

    let threshold_label = NSTextField::labelWithString(ns_string!("Avisar a partir de"), mtm);
    threshold_label.setFrame(NSRect::new(
        NSPoint::new(44.0, 70.0),
        NSSize::new(180.0, 24.0),
    ));

    let threshold = unsafe {
        let popup = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            NSRect::new(NSPoint::new(245.0, 66.0), NSSize::new(110.0, 30.0)),
            false,
        );
        popup.setTarget(Some(target as &AnyObject));
        popup.setAction(Some(sel!(changeWarningThreshold:)));
        popup
    };
    for value in [70, 75, 80, 85, 90, 95] {
        threshold.addItemWithTitle(&threshold_title(
            WarningThreshold::try_from(value).expect("known threshold"),
        ));
    }
    threshold.setAccessibilityLabel(Some(ns_string!("Limite de aviso do disco")));
    threshold.setAccessibilityHelp(Some(ns_string!(
        "Escolha o percentual de ocupação que inicia a observação de pouco espaço."
    )));

    content.addSubview(&heading);
    content.addSubview(&checkbox);
    content.addSubview(&explanation);
    content.addSubview(&threshold_label);
    content.addSubview(&threshold);
    window.setInitialFirstResponder(Some(&checkbox));

    PreferencesWindow {
        window,
        mole_checkbox: checkbox,
        warning_threshold: threshold,
    }
}

fn create_free_space_window(mtm: MainThreadMarker, target: &ControlTarget) -> FreeSpaceWindow {
    let window = create_window(mtm, "Liberar espaço", NSSize::new(540.0, 420.0));
    let content = window
        .contentView()
        .expect("free-space window content view");

    let heading = text_label(
        mtm,
        "Liberar espaço",
        NSRect::new(NSPoint::new(24.0, 360.0), NSSize::new(490.0, 28.0)),
    );
    heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(18.0)));
    let volume_name = NSFileManager::defaultManager()
        .displayNameAtPath(ns_string!("/"))
        .to_string();
    let subtitle = text_label(
        mtm,
        &format!("Volume de inicialização: {volume_name}"),
        NSRect::new(NSPoint::new(24.0, 330.0), NSSize::new(490.0, 24.0)),
    );
    subtitle.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));

    let occupied_label = text_label(
        mtm,
        "Ocupado",
        NSRect::new(NSPoint::new(24.0, 278.0), NSSize::new(240.0, 24.0)),
    );
    let occupied_value = value_label(mtm, 278.0);
    let available_label = text_label(
        mtm,
        "Disponível para uso importante",
        NSRect::new(NSPoint::new(24.0, 242.0), NSSize::new(270.0, 24.0)),
    );
    let available_value = value_label(mtm, 242.0);
    let threshold_label = text_label(
        mtm,
        "Limite configurado",
        NSRect::new(NSPoint::new(24.0, 206.0), NSSize::new(240.0, 24.0)),
    );
    let threshold_value = value_label(mtm, 206.0);

    let guarantee = text_label(
        mtm,
        "O Statlet apenas monitora e avisa. Abrir esta janela não analisa nem remove arquivos; o macOS pode recuperar parte do espaço disponível.",
        NSRect::new(NSPoint::new(24.0, 142.0), NSSize::new(490.0, 48.0)),
    );
    guarantee.setMaximumNumberOfLines(3);
    guarantee.setUsesSingleLineMode(false);
    guarantee.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    guarantee.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));

    let mole_status = text_label(
        mtm,
        "Verificando a instalação do Mole…",
        NSRect::new(NSPoint::new(24.0, 88.0), NSSize::new(490.0, 42.0)),
    );
    mole_status.setMaximumNumberOfLines(2);

    let open_mole_button = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Abrir Mole no Terminal"),
            Some(target as &AnyObject),
            Some(sel!(openMoleInTerminal:)),
            mtm,
        )
    };
    open_mole_button.setFrame(NSRect::new(
        NSPoint::new(24.0, 28.0),
        NSSize::new(190.0, 34.0),
    ));
    open_mole_button.setEnabled(false);
    open_mole_button.setAccessibilityLabel(Some(ns_string!("Abrir Mole no Terminal")));
    open_mole_button.setAccessibilityHelp(Some(ns_string!(
        "Abre o comando interativo oficial do Mole fora do Statlet."
    )));

    content.addSubview(&heading);
    content.addSubview(&subtitle);
    content.addSubview(&occupied_label);
    content.addSubview(&occupied_value);
    content.addSubview(&available_label);
    content.addSubview(&available_value);
    content.addSubview(&threshold_label);
    content.addSubview(&threshold_value);
    content.addSubview(&guarantee);
    content.addSubview(&mole_status);
    content.addSubview(&open_mole_button);
    window.setInitialFirstResponder(Some(&open_mole_button));

    FreeSpaceWindow {
        window,
        occupied_value,
        available_value,
        threshold_value,
        mole_status,
        open_mole_button,
    }
}

fn text_label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(text), mtm);
    label.setFrame(frame);
    label
}

fn value_label(mtm: MainThreadMarker, y: f64) -> Retained<NSTextField> {
    let label = text_label(
        mtm,
        "—",
        NSRect::new(NSPoint::new(320.0, y), NSSize::new(194.0, 24.0)),
    );
    label.setAlignment(objc2_app_kit::NSTextAlignment::Right);
    label.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(13.0)));
    label
}

impl HistoryWindow {
    fn apply(&self, history: &History) {
        let empty = history.is_empty();
        self.empty_label.setHidden(!empty);
        self.scroll_view.setHidden(empty);
        self.clear_button.setEnabled(!empty);

        let document_height = (history.records().len() as f64 * 36.0).max(320.0);
        self.document
            .setFrameSize(NSSize::new(532.0, document_height));
        for (index, row) in self.rows.iter().enumerate() {
            let Some(record) = history.records().get(index) else {
                row.setHidden(true);
                continue;
            };
            row.setHidden(false);
            row.setStringValue(&objc2_foundation::NSString::from_str(
                &format_history_record(*record),
            ));
            row.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(
                &format_history_record(*record),
            )));
            row.setFrame(NSRect::new(
                NSPoint::new(8.0, document_height - ((index + 1) as f64 * 36.0)),
                NSSize::new(508.0, 28.0),
            ));
        }
        let clip_view = self.scroll_view.contentView();
        clip_view.scrollToPoint(NSPoint::new(0.0, (document_height - 300.0).max(0.0)));
        self.scroll_view.reflectScrolledClipView(&clip_view);
    }
}

fn create_history_window(mtm: MainThreadMarker, target: &ControlTarget) -> HistoryWindow {
    let window = create_window(mtm, "Histórico do Statlet", NSSize::new(600.0, 480.0));
    let content = window.contentView().expect("history window content view");

    let heading = NSTextField::labelWithString(ns_string!("Histórico local"), mtm);
    heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(17.0)));
    heading.setFrame(NSRect::new(
        NSPoint::new(24.0, 420.0),
        NSSize::new(552.0, 28.0),
    ));

    let explanation = NSTextField::labelWithString(
        ns_string!("Até 30 eventos do disco e da integração, mantidos somente neste Mac."),
        mtm,
    );
    explanation.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    explanation.setFrame(NSRect::new(
        NSPoint::new(24.0, 390.0),
        NSSize::new(552.0, 24.0),
    ));

    let empty_label = NSTextField::labelWithString(
        ns_string!(
            "Nenhum evento registrado. O Statlet não registra nomes nem caminhos de arquivos."
        ),
        mtm,
    );
    empty_label.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    empty_label.setFrame(NSRect::new(
        NSPoint::new(24.0, 220.0),
        NSSize::new(552.0, 44.0),
    ));
    empty_label.setMaximumNumberOfLines(2);
    empty_label.setUsesSingleLineMode(false);
    empty_label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);

    let scroll_view = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(NSPoint::new(24.0, 76.0), NSSize::new(552.0, 300.0)),
    );
    scroll_view.setHasVerticalScroller(true);
    scroll_view.setDrawsBackground(false);
    scroll_view.setAccessibilityLabel(Some(ns_string!(
        "Eventos do histórico local, do mais recente para o mais antigo"
    )));
    let document = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(532.0, 320.0)),
    );
    let mut rows = Vec::with_capacity(MAX_HISTORY_RECORDS);
    for _ in 0..MAX_HISTORY_RECORDS {
        let row = text_label(
            mtm,
            "",
            NSRect::new(NSPoint::new(8.0, 0.0), NSSize::new(508.0, 28.0)),
        );
        row.setHidden(true);
        row.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        document.addSubview(&row);
        rows.push(row);
    }
    scroll_view.setDocumentView(Some(&document));

    let clear_button = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Apagar histórico…"),
            Some(target as &AnyObject),
            Some(sel!(clearHistory:)),
            mtm,
        )
    };
    clear_button.setFrame(NSRect::new(
        NSPoint::new(24.0, 24.0),
        NSSize::new(160.0, 34.0),
    ));
    clear_button.setEnabled(false);
    clear_button.setAccessibilityLabel(Some(ns_string!("Apagar histórico local")));
    clear_button.setAccessibilityHelp(Some(ns_string!(
        "Pede confirmação antes de remover todos os eventos locais do Statlet."
    )));

    content.addSubview(&heading);
    content.addSubview(&explanation);
    content.addSubview(&empty_label);
    content.addSubview(&scroll_view);
    content.addSubview(&clear_button);
    window.setInitialFirstResponder(Some(&clear_button));

    HistoryWindow {
        window,
        document,
        rows,
        empty_label,
        scroll_view,
        clear_button,
    }
}

fn format_history_record(record: HistoryRecord) -> String {
    let date = NSDate::dateWithTimeIntervalSince1970(record.timestamp_unix_seconds as f64);
    let timestamp = NSDateFormatter::localizedStringFromDate_dateStyle_timeStyle(
        &date,
        NSDateFormatterStyle::ShortStyle,
        NSDateFormatterStyle::ShortStyle,
    );
    let summary = match record.kind {
        HistoryEventKind::DiskPressureStarted => "Pouco espaço detectado",
        HistoryEventKind::DiskPressureRecovered => "Uso do disco voltou ao normal",
        HistoryEventKind::MoleMissing => "Mole não encontrado",
        HistoryEventKind::MoleIncompatible => "Versão do Mole incompatível",
        HistoryEventKind::MoleUnavailable => "Não foi possível verificar o Mole",
        HistoryEventKind::MonitoringFailed => "Falha ao ler o volume de inicialização",
    };
    format!("{timestamp}  —  {summary}")
}

fn create_window(mtm: MainThreadMarker, title: &str, size: NSSize) -> Retained<NSWindow> {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), size),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);
    window.setTitle(&objc2_foundation::NSString::from_str(title));
    window.center();
    window
}

fn threshold_title(threshold: WarningThreshold) -> Retained<objc2_foundation::NSString> {
    objc2_foundation::NSString::from_str(&format!("{}%", threshold.get()))
}

#[cfg(test)]
mod tests {
    use super::{
        detail_label_display_text, detail_label_frame, detail_value_display_text,
        focused_process_disappearance_announcement, memory_composition_stroke_style,
        process_cell_presentation, process_selection_after_update, process_table_column_widths,
        system_usage_layout,
    };
    use objc2_foundation::NSSize;
    use statlet::stats::ProcessRowViewModel;

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
        let layout = system_usage_layout(NSSize::new(620.0, 520.0));
        let max_x = |frame: objc2_foundation::NSRect| frame.origin.x + frame.size.width;
        let max_y = |frame: objc2_foundation::NSRect| frame.origin.y + frame.size.height;

        assert!(max_x(layout.segmented) <= 620.0);
        assert!(max_y(layout.segmented) <= 520.0);
        assert!(max_x(layout.primary) < layout.detail_labels[0].origin.x);
        assert!(max_x(layout.memory_composition) <= 620.0);
        assert!(max_y(layout.detail_labels[0]) <= layout.memory_composition.origin.y);
        assert!(max_y(layout.memory_composition) < layout.segmented.origin.y);
        assert!(max_x(layout.history_heading) < layout.detail_labels[4].origin.x);
        assert!(max_y(layout.graph) < layout.detail_values[5].origin.y);
        assert!(max_y(layout.history_summary) < layout.graph.origin.y);
        assert!(max_y(layout.process_heading) < layout.history_summary.origin.y);
        assert!(max_y(layout.process_scroll) < layout.process_heading.origin.y);
        assert!(layout.process_scroll.size.height >= 50.0);
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
