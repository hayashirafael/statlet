use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBezierPath, NSButton, NSColor,
    NSControlStateValueOn, NSControlTextEditingDelegate, NSEventModifierFlags, NSLineBreakMode,
    NSPopUpButton, NSScrollView, NSSegmentSwitchTracking, NSSegmentedControl, NSTableColumn,
    NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTextField, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSDate, NSDateFormatter, NSDateFormatterStyle,
    NSFileManager, NSIndexSet, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use statlet::core::{AppEvent, AppState, Preferences, WarningThreshold, WindowKind};
use statlet::disk::format_decimal_gigabytes;
use statlet::history::{History, HistoryEventKind, HistoryRecord, MAX_HISTORY_RECORDS};
use statlet::mole::MoleStatus;
use statlet::stats::{
    history_x_position, ProcessRowViewModel, SystemUsageSection, SystemUsageViewModel, UsagePoint,
};
use tao::event_loop::EventLoopProxy;

use super::RuntimeEvent;

struct UsageGraphViewIvars {
    points: RefCell<Vec<UsagePoint>>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = UsageGraphViewIvars]
    struct UsageGraphView;

    unsafe impl NSObjectProtocol for UsageGraphView {}

    impl UsageGraphView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let points = self.ivars().points.borrow();
            if points.len() < 2 {
                return;
            }
            let path = NSBezierPath::new();
            let width = (bounds.size.width - 4.0).max(1.0);
            let height = (bounds.size.height - 4.0).max(1.0);
            let window_end = points.last().expect("at least two points").observed_at;
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
    }
);

impl UsageGraphView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(UsageGraphViewIvars {
            points: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn apply(&self, points: &[UsagePoint], accessibility_label: &str) {
        self.ivars().points.replace(points.to_vec());
        self.setAccessibilityElement(true);
        self.setAccessibilityLabel(Some(&NSString::from_str(accessibility_label)));
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
                    &NSString::from_str(text),
                    MainThreadMarker::new().expect("table cells are created on the main thread"),
                );
                field.setAccessibilityLabel(Some(&NSString::from_str(accessibility_label)));
                if column == "memory" {
                    field.setAlignment(objc2_app_kit::NSTextAlignment::Right);
                }
                Some(Retained::into_super(Retained::into_super(field)))
            })()
        }
    }
);

fn process_cell_presentation<'a>(row: &'a ProcessRowViewModel, column: &str) -> (&'a str, &'a str) {
    let text = if column == "memory" {
        row.memory.as_str()
    } else {
        row.name.as_str()
    };
    (text, row.accessibility_label.as_str())
}

impl StatsTableDataSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(StatsTableDataSourceIvars {
            rows: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn replace_rows(&self, rows: &[ProcessRowViewModel]) {
        self.ivars().rows.replace(rows.to_vec());
    }

    fn pid_at(&self, row: isize) -> Option<u32> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.ivars().rows.borrow().get(row).map(|row| row.pid))
    }

    fn index_of_pid(&self, pid: u32) -> Option<usize> {
        self.ivars()
            .rows
            .borrow()
            .iter()
            .position(|row| row.pid == pid)
    }
}

struct ControlTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControlTargetIvars]
    struct ControlTarget;

    unsafe impl NSObjectProtocol for ControlTarget {}

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
            let _ = self.ivars().proxy.send_event(RuntimeEvent::App(
                AppEvent::SelectSystemUsageSection(section),
            ));
        }

        #[unsafe(method(selectSystemUsageRam:))]
        fn select_system_usage_ram(&self, _sender: &NSButton) {
            let _ = self.ivars().proxy.send_event(RuntimeEvent::App(
                AppEvent::SelectSystemUsageSection(SystemUsageSection::Ram),
            ));
        }

        #[unsafe(method(selectSystemUsageGpu:))]
        fn select_system_usage_gpu(&self, _sender: &NSButton) {
            let _ = self.ivars().proxy.send_event(RuntimeEvent::App(
                AppEvent::SelectSystemUsageSection(SystemUsageSection::Gpu),
            ));
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
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ControlTargetIvars { proxy });
        unsafe { msg_send![super(this), init] }
    }
}

pub struct WindowManager {
    control_target: Retained<ControlTarget>,
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
    graph: Retained<UsageGraphView>,
    history_summary: Retained<NSTextField>,
    detail_labels: Vec<Retained<NSTextField>>,
    detail_values: Vec<Retained<NSTextField>>,
    process_heading: Retained<NSTextField>,
    process_scroll: Retained<NSScrollView>,
    process_table: Retained<NSTableView>,
    process_data_source: Retained<StatsTableDataSource>,
}

struct SystemUsageLayout {
    segmented: NSRect,
    primary: NSRect,
    secondary: NSRect,
    status: NSRect,
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

impl WindowManager {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let control_target = ControlTarget::new(mtm, proxy);
        Self {
            control_target,
            preferences: None,
            history: None,
            free_space: None,
            system_usage: None,
        }
    }

    pub fn show(&mut self, kind: WindowKind, state: &AppState, history: &History) {
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
            .is_some_and(|window| window.window.isVisible())
    }

    pub fn update_system_usage(&self, view_model: &SystemUsageViewModel) {
        if let Some(window) = &self.system_usage {
            window.apply(view_model);
        }
    }
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
    fn apply(&self, view_model: &SystemUsageViewModel) {
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
            label.setHidden(false);
            label.setStringValue(&NSString::from_str(detail.label));
            self.detail_values[index].setHidden(false);
            self.detail_values[index].setStringValue(&NSString::from_str(&detail.value));
            self.detail_values[index].setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{}: {}",
                detail.label, detail.value
            ))));
        }

        let shows_processes = view_model.section == SystemUsageSection::Ram;
        self.process_heading.setHidden(!shows_processes);
        self.process_scroll.setHidden(!shows_processes);
        let selected_pid = self
            .process_data_source
            .pid_at(self.process_table.selectedRow());
        let clip_view = self.process_scroll.contentView();
        let scroll_origin = clip_view.bounds().origin;
        self.process_data_source
            .replace_rows(&view_model.process_rows);
        self.process_table.reloadData();
        clip_view.scrollToPoint(scroll_origin);
        self.process_scroll.reflectScrolledClipView(&clip_view);
        if let Some(index) = selected_pid.and_then(|pid| self.process_data_source.index_of_pid(pid))
        {
            self.process_table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
        }
    }
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
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(612.0, 188.0)),
    );
    let process_column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("process"));
    process_column.setWidth(440.0);
    process_column
        .headerCell()
        .setStringValue(ns_string!("Processo"));
    let memory_column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("memory"));
    memory_column.setWidth(160.0);
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
        graph,
        history_summary,
        detail_labels,
        detail_values,
        process_heading,
        process_scroll,
        process_table,
        process_data_source,
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
    use super::{process_cell_presentation, system_usage_layout};
    use objc2_foundation::NSSize;
    use statlet::stats::ProcessRowViewModel;

    #[test]
    fn native_process_cells_use_the_complete_row_accessibility_label() {
        let row = ProcessRowViewModel {
            pid: 42,
            name: "Safari".to_owned(),
            memory: "512 MB".to_owned(),
            accessibility_label: "Safari, 512 MB".to_owned(),
        };

        assert_eq!(
            process_cell_presentation(&row, "process"),
            ("Safari", "Safari, 512 MB")
        );
        assert_eq!(
            process_cell_presentation(&row, "memory"),
            ("512 MB", "Safari, 512 MB")
        );
    }

    #[test]
    fn system_usage_layout_fits_without_overlap_at_the_declared_minimum() {
        let layout = system_usage_layout(NSSize::new(620.0, 520.0));
        let max_x = |frame: objc2_foundation::NSRect| frame.origin.x + frame.size.width;
        let max_y = |frame: objc2_foundation::NSRect| frame.origin.y + frame.size.height;

        assert!(max_x(layout.segmented) <= 620.0);
        assert!(max_y(layout.segmented) <= 520.0);
        assert!(max_x(layout.primary) < layout.detail_labels[0].origin.x);
        assert!(max_x(layout.history_heading) < layout.detail_labels[4].origin.x);
        assert!(max_y(layout.graph) < layout.detail_values[5].origin.y);
        assert!(max_y(layout.history_summary) < layout.graph.origin.y);
        assert!(max_y(layout.process_heading) < layout.history_summary.origin.y);
        assert!(max_y(layout.process_scroll) < layout.process_heading.origin.y);
        assert!(layout.process_scroll.size.height >= 50.0);
    }
}
