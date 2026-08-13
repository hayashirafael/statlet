use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly, Message};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColor, NSColorWell, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSFont, NSSegmentSwitchTracking, NSSegmentedControl, NSStackView,
    NSStepper, NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize,
};
use statlet::core::{AppEvent, IndicatorPreferenceChange};
use statlet::indicator_preferences::{
    FontFamilyPreference, FontSize, FontWeight, IndicatorPreferenceGroup, IndicatorPreferences,
    LabelColorMode, MetricColorMode, MetricKind, TypographyPreferences,
};
use statlet::preferences_view::{
    ColorEditorState, IndicatorControlsLayout, IndicatorControlsVisibility, IntervalDraft,
};
use tao::event_loop::EventLoopProxy;

use super::color_editor::{ColorBinding, ColorEditor};
use super::font_picker::FontPicker;
use super::{IndicatorFontFallback, IndicatorLayoutDiagnostics};
use crate::macos::fonts::FontCatalog;
use crate::macos::RuntimeEvent;

struct IndicatorControlsTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    applying: Cell<bool>,
    selected_family: RefCell<FontFamilyPreference>,
    selected_font_size: Cell<FontSize>,
    interval_draft: RefCell<IntervalDraft>,
    font_resources: RefCell<Option<FontResources>>,
    font_size: Retained<NSTextField>,
    interval_field: Retained<NSTextField>,
    interval_stepper: Retained<NSStepper>,
    interval_error: Retained<NSTextField>,
}

struct FontResources {
    picker: FontPicker,
    catalog: FontCatalog,
}

fn get_or_create_font_resources<T>(slot: &mut Option<T>, create: impl FnOnce() -> T) -> &mut T {
    slot.get_or_insert_with(create)
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = IndicatorControlsTargetIvars]
    struct IndicatorControlsTarget;

    unsafe impl NSObjectProtocol for IndicatorControlsTarget {}

    impl IndicatorControlsTarget {
        #[unsafe(method(changeCpuColorMode:))]
        fn change_cpu_color_mode(&self, sender: &NSSegmentedControl) {
            self.send_metric_mode(MetricKind::Cpu, sender);
        }

        #[unsafe(method(resetCpuAndRam:))]
        fn reset_cpu_and_ram(&self, _sender: &NSButton) {
            self.reset_group(IndicatorPreferenceGroup::CpuAndRam);
        }

        #[unsafe(method(changeRamColorMode:))]
        fn change_ram_color_mode(&self, sender: &NSSegmentedControl) {
            self.send_metric_mode(MetricKind::Ram, sender);
        }

        #[unsafe(method(toggleLabelsVisible:))]
        fn toggle_labels_visible(&self, sender: &NSButton) {
            if self.ivars().applying.get() {
                return;
            }
            self.send(IndicatorPreferenceChange::SetLabelsVisible(
                sender.state() == NSControlStateValueOn,
            ));
        }

        #[unsafe(method(changeLabelColorMode:))]
        fn change_label_color_mode(&self, sender: &NSSegmentedControl) {
            if self.ivars().applying.get() {
                return;
            }
            let mode = match sender.selectedSegment() {
                1 => LabelColorMode::MatchMetric,
                2 => LabelColorMode::Fixed,
                _ => LabelColorMode::Neutral,
            };
            self.send(IndicatorPreferenceChange::SetLabelColorMode(mode));
        }

        #[unsafe(method(resetLabels:))]
        fn reset_labels(&self, _sender: &NSButton) {
            self.reset_group(IndicatorPreferenceGroup::Labels);
        }

        #[unsafe(method(openFontPicker:))]
        fn open_font_picker(&self, sender: &NSButton) {
            let Some(parent) = sender.window() else {
                return;
            };
            let selected = self.ivars().selected_family.borrow().clone();
            let marker = MainThreadMarker::new().expect("font picker actions run on main thread");
            let proxy = self.ivars().proxy.clone();
            let mut slot = self.ivars().font_resources.borrow_mut();
            let resources = get_or_create_font_resources(&mut slot, || FontResources {
                picker: FontPicker::new(marker, proxy),
                catalog: FontCatalog::new(marker),
            });
            resources.catalog.refresh();
            resources.picker.refresh_catalog(&resources.catalog);
            resources
                .picker
                .present(&parent, &resources.catalog, &selected);
        }

        #[unsafe(method(commitFontSize:))]
        fn commit_font_size_action(&self, sender: &NSTextField) {
            self.commit_font_size(sender);
        }

        #[unsafe(method(changeFontWeight:))]
        fn change_font_weight(&self, sender: &NSSegmentedControl) {
            if self.ivars().applying.get() {
                return;
            }
            let weight = match sender.selectedSegment() {
                0 => FontWeight::Regular,
                2 => FontWeight::Bold,
                _ => FontWeight::Medium,
            };
            self.send(IndicatorPreferenceChange::SetFontWeight(weight));
        }

        #[unsafe(method(resetTypography:))]
        fn reset_typography(&self, _sender: &NSButton) {
            self.reset_group(IndicatorPreferenceGroup::Typography);
        }

        #[unsafe(method(commitRefreshInterval:))]
        fn commit_refresh_interval_action(&self, sender: &NSTextField) {
            self.commit_refresh_interval(sender);
        }

        #[unsafe(method(stepRefreshInterval:))]
        fn step_refresh_interval(&self, sender: &NSStepper) {
            if self.ivars().applying.get() {
                return;
            }
            let text = sender.integerValue().to_string();
            self.commit_refresh_interval_text(&text);
        }

        #[unsafe(method(resetRefreshInterval:))]
        fn reset_refresh_interval(&self, _sender: &NSButton) {
            self.reset_group(IndicatorPreferenceGroup::RefreshInterval);
        }
    }

    unsafe impl NSControlTextEditingDelegate for IndicatorControlsTarget {
        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, notification: &NSNotification) {
            if self.ivars().applying.get() {
                return;
            }
            let Some(field) = notification_text_field(notification) else {
                return;
            };
            if std::ptr::eq(&*field, &*self.ivars().font_size) {
                self.commit_font_size(&field);
            } else if std::ptr::eq(&*field, &*self.ivars().interval_field) {
                self.commit_refresh_interval(&field);
            }
        }
    }

    unsafe impl NSTextFieldDelegate for IndicatorControlsTarget {}
);

impl IndicatorControlsTarget {
    fn new(mtm: MainThreadMarker, ivars: IndicatorControlsTargetIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    fn send_metric_mode(&self, metric: MetricKind, sender: &NSSegmentedControl) {
        if self.ivars().applying.get() {
            return;
        }
        let mode = if sender.selectedSegment() == 1 {
            MetricColorMode::Fixed
        } else {
            MetricColorMode::Dynamic
        };
        self.send(IndicatorPreferenceChange::SetMetricColorMode { metric, mode });
    }

    fn send(&self, change: IndicatorPreferenceChange) {
        self.send_event(AppEvent::UpdateIndicator(change));
    }

    fn send_event(&self, event: AppEvent) {
        let _ = self.ivars().proxy.send_event(RuntimeEvent::App(event));
    }

    fn reset_group(&self, group: IndicatorPreferenceGroup) {
        if !self.ivars().applying.get() {
            self.send_event(AppEvent::ResetIndicatorGroup(group));
        }
    }

    fn commit_font_size(&self, field: &NSTextField) {
        if self.ivars().applying.get() {
            return;
        }
        let size = field
            .stringValue()
            .to_string()
            .trim()
            .parse::<u8>()
            .ok()
            .and_then(|points| FontSize::try_from(points).ok());
        let Some(size) = size else {
            field.setStringValue(&objc2_foundation::NSString::from_str(
                &self.ivars().selected_font_size.get().points().to_string(),
            ));
            return;
        };
        field.setStringValue(&objc2_foundation::NSString::from_str(
            &size.points().to_string(),
        ));
        if size != self.ivars().selected_font_size.replace(size) {
            self.send(IndicatorPreferenceChange::SetFontSize(size));
        }
    }

    fn commit_refresh_interval(&self, field: &NSTextField) {
        self.commit_refresh_interval_text(&field.stringValue().to_string());
    }

    fn commit_refresh_interval_text(&self, text: &str) {
        let previous = self.ivars().interval_draft.borrow().valid_interval();
        let result = self.ivars().interval_draft.borrow_mut().commit(text);
        match result {
            Ok(interval) => {
                self.ivars()
                    .interval_field
                    .setStringValue(&objc2_foundation::NSString::from_str(
                        &interval.seconds().to_string(),
                    ));
                self.ivars()
                    .interval_stepper
                    .setIntegerValue(interval.seconds().into());
                set_inline_error(&self.ivars().interval_error, None);
                if interval != previous {
                    self.send(IndicatorPreferenceChange::SetRefreshInterval(interval));
                }
            }
            Err(error) => set_inline_error(&self.ivars().interval_error, Some(error.message())),
        }
    }
}

struct IndicatorLayoutViews {
    colors_heading: Retained<NSTextField>,
    reset_cpu_and_ram: Retained<NSButton>,
    cpu_label: Retained<NSTextField>,
    cpu_mode: Retained<NSSegmentedControl>,
    cpu_editor: Retained<NSStackView>,
    ram_label: Retained<NSTextField>,
    ram_mode: Retained<NSSegmentedControl>,
    ram_editor: Retained<NSStackView>,
    labels_heading: Retained<NSTextField>,
    labels_visible: Retained<NSButton>,
    labels_mode: Retained<NSSegmentedControl>,
    reset_labels: Retained<NSButton>,
    labels_editor: Retained<NSStackView>,
    typography_heading: Retained<NSTextField>,
    family_label: Retained<NSTextField>,
    font_family: Retained<NSButton>,
    size_label: Retained<NSTextField>,
    font_size: Retained<NSTextField>,
    points_label: Retained<NSTextField>,
    weight_label: Retained<NSTextField>,
    font_weight: Retained<NSSegmentedControl>,
    reset_typography: Retained<NSButton>,
    font_fallback_warning: Retained<NSTextField>,
    layout_warning: Retained<NSTextField>,
    update_heading: Retained<NSTextField>,
    interval_label: Retained<NSTextField>,
    interval_field: Retained<NSTextField>,
    interval_stepper: Retained<NSStepper>,
    seconds_label: Retained<NSTextField>,
    reset_refresh_interval: Retained<NSButton>,
    interval_help: Retained<NSTextField>,
    interval_error: Retained<NSTextField>,
}

impl IndicatorLayoutViews {
    fn apply(&self, layout: &IndicatorControlsLayout) {
        let content_height = layout.content_height();

        let colors_heading = layout.colors_heading();
        self.colors_heading
            .setFrameOrigin(NSPoint::new(0.0, colors_heading.origin_y(content_height)));
        let colors_reset = layout.colors_reset();
        self.reset_cpu_and_ram.setFrameOrigin(NSPoint::new(
            colors_reset.x(),
            colors_reset.origin_y(content_height),
        ));

        let cpu_row = layout.cpu_row();
        self.cpu_label.setFrameOrigin(NSPoint::new(
            cpu_row.label_x(),
            cpu_row.label_origin_y(content_height),
        ));
        self.cpu_mode.setFrameOrigin(NSPoint::new(
            cpu_row.control_x(),
            cpu_row.control_origin_y(content_height),
        ));
        if let Some(cpu_editor) = layout.cpu_editor() {
            self.cpu_editor.setFrame(NSRect::new(
                NSPoint::new(0.0, cpu_editor.origin_y(content_height)),
                NSSize::new(540.0, cpu_editor.height()),
            ));
        }

        let ram_row = layout.ram_row();
        self.ram_label.setFrameOrigin(NSPoint::new(
            ram_row.label_x(),
            ram_row.label_origin_y(content_height),
        ));
        self.ram_mode.setFrameOrigin(NSPoint::new(
            ram_row.control_x(),
            ram_row.control_origin_y(content_height),
        ));
        if let Some(ram_editor) = layout.ram_editor() {
            self.ram_editor.setFrame(NSRect::new(
                NSPoint::new(0.0, ram_editor.origin_y(content_height)),
                NSSize::new(540.0, ram_editor.height()),
            ));
        }

        let labels_heading = layout.labels_heading();
        self.labels_heading
            .setFrameOrigin(NSPoint::new(0.0, labels_heading.origin_y(content_height)));
        let labels_visibility = layout.labels_visibility_row();
        let labels_visibility_y = labels_visibility.origin_y(content_height);
        self.labels_visible.setFrameOrigin(NSPoint::new(
            labels_visibility.label_x(),
            labels_visibility_y,
        ));
        self.reset_labels
            .setFrameOrigin(NSPoint::new(390.0, labels_visibility_y));
        let labels_mode = layout.labels_mode_row();
        self.labels_mode.setFrameOrigin(NSPoint::new(
            labels_mode.control_x(),
            labels_mode.control_origin_y(content_height),
        ));
        if let Some(labels_editor) = layout.labels_editor() {
            self.labels_editor.setFrame(NSRect::new(
                NSPoint::new(0.0, labels_editor.origin_y(content_height)),
                NSSize::new(540.0, labels_editor.height()),
            ));
        }

        let typography_heading = layout.typography_heading();
        self.typography_heading.setFrameOrigin(NSPoint::new(
            0.0,
            typography_heading.origin_y(content_height),
        ));
        let family_row = layout.family_row();
        let family_y = family_row.origin_y(content_height);
        self.family_label
            .setFrameOrigin(NSPoint::new(family_row.label_x(), family_y));
        self.font_family
            .setFrameOrigin(NSPoint::new(family_row.control_x(), family_y));
        let size_row = layout.size_row();
        let size_y = size_row.origin_y(content_height);
        self.size_label
            .setFrameOrigin(NSPoint::new(size_row.label_x(), size_y));
        self.font_size
            .setFrameOrigin(NSPoint::new(size_row.control_x(), size_y));
        self.points_label
            .setFrameOrigin(NSPoint::new(166.0, size_y));
        let weight_row = layout.weight_row();
        let weight_y = weight_row.origin_y(content_height);
        self.weight_label
            .setFrameOrigin(NSPoint::new(weight_row.label_x(), weight_y));
        self.font_weight
            .setFrameOrigin(NSPoint::new(weight_row.control_x(), weight_y));
        self.reset_typography
            .setFrameOrigin(NSPoint::new(400.0, weight_y));
        self.font_fallback_warning.setFrameOrigin(NSPoint::new(
            100.0,
            layout.font_fallback_warning().origin_y(content_height),
        ));
        self.layout_warning.setFrameOrigin(NSPoint::new(
            100.0,
            layout.layout_warning().origin_y(content_height),
        ));

        let update_heading = layout.update_heading();
        self.update_heading
            .setFrameOrigin(NSPoint::new(0.0, update_heading.origin_y(content_height)));
        let interval_row = layout.interval_row();
        let interval_y = interval_row.origin_y(content_height);
        self.interval_label
            .setFrameOrigin(NSPoint::new(interval_row.label_x(), interval_y));
        self.interval_field
            .setFrameOrigin(NSPoint::new(interval_row.control_x(), interval_y));
        self.interval_stepper
            .setFrameOrigin(NSPoint::new(164.0, interval_y));
        self.seconds_label
            .setFrameOrigin(NSPoint::new(194.0, interval_y));
        self.reset_refresh_interval
            .setFrameOrigin(NSPoint::new(350.0, interval_y));
        self.interval_help.setFrameOrigin(NSPoint::new(
            0.0,
            layout.interval_help().origin_y(content_height),
        ));
        self.interval_error.setFrameOrigin(NSPoint::new(
            100.0,
            layout.interval_error().origin_y(content_height),
        ));
    }
}

pub(super) struct IndicatorControls {
    view: Retained<NSStackView>,
    layout_views: IndicatorLayoutViews,
    content_height: Cell<f64>,
    cpu_mode: Retained<NSSegmentedControl>,
    reset_cpu_and_ram: Retained<NSButton>,
    ram_mode: Retained<NSSegmentedControl>,
    labels_visible: Retained<NSButton>,
    labels_mode: Retained<NSSegmentedControl>,
    reset_labels: Retained<NSButton>,
    font_family: Retained<NSButton>,
    font_size: Retained<NSTextField>,
    font_weight: Retained<NSSegmentedControl>,
    reset_typography: Retained<NSButton>,
    interval_field: Retained<NSTextField>,
    interval_stepper: Retained<NSStepper>,
    reset_refresh_interval: Retained<NSButton>,
    cpu_editor: ColorEditor,
    ram_editor: ColorEditor,
    labels_editor: ColorEditor,
    target: Retained<IndicatorControlsTarget>,
}

impl IndicatorControls {
    pub(super) fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let view = NSStackView::initWithFrame(
            NSStackView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 0.0)),
        );

        let colors_heading = heading(mtm, "Cores", 0.0);
        let cpu_label = text_label(mtm, "CPU", 0.0);
        let placeholder_target = None;
        let cpu_mode = segmented(
            mtm,
            &["Dinâmica", "Fixa"],
            "indicator.cpu.mode",
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(240.0, 28.0)),
            placeholder_target,
            None,
        );
        let reset_cpu_and_ram = reset_button(
            mtm,
            "Restaurar CPU e RAM",
            NSRect::new(NSPoint::new(390.0, 0.0), NSSize::new(160.0, 28.0)),
        );
        let cpu_editor = ColorEditor::new(
            mtm,
            ColorBinding::MetricShared(MetricKind::Cpu),
            proxy.clone(),
        );
        cpu_editor.view().setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(540.0, 160.0),
        ));

        let ram_label = text_label(mtm, "RAM", 0.0);
        let ram_mode = segmented(
            mtm,
            &["Dinâmica", "Fixa"],
            "indicator.ram.mode",
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(240.0, 28.0)),
            placeholder_target,
            None,
        );
        let ram_editor = ColorEditor::new(
            mtm,
            ColorBinding::MetricShared(MetricKind::Ram),
            proxy.clone(),
        );
        ram_editor.view().setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(540.0, 160.0),
        ));

        let labels_heading = heading(mtm, "Rótulos", 0.0);
        let labels_visible = unsafe {
            NSButton::checkboxWithTitle_target_action(
                ns_string!("Mostrar rótulos C/R"),
                None,
                None,
                mtm,
            )
        };
        labels_visible.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(220.0, 24.0),
        ));
        labels_visible.setAccessibilityIdentifier(Some(ns_string!("indicator.labels.visible")));
        let labels_mode = segmented(
            mtm,
            &["Neutra", "Igual ao valor", "Personalizada"],
            "indicator.labels.mode",
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(390.0, 28.0)),
            placeholder_target,
            None,
        );
        let reset_labels = reset_button(
            mtm,
            "Restaurar rótulos",
            NSRect::new(NSPoint::new(390.0, 0.0), NSSize::new(160.0, 28.0)),
        );
        let labels_editor = ColorEditor::new(mtm, ColorBinding::LabelShared, proxy.clone());
        labels_editor.view().setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(540.0, 160.0),
        ));

        let typography_heading = heading(mtm, "Tipografia", 0.0);
        let family_label = text_label(mtm, "Família", 0.0);
        let font_family = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("System Monospaced"),
                None,
                None,
                mtm,
            )
        };
        font_family.setFrame(NSRect::new(
            NSPoint::new(100.0, 0.0),
            NSSize::new(300.0, 30.0),
        ));
        font_family.setAccessibilityIdentifier(Some(ns_string!("indicator.font.family")));

        let size_label = text_label(mtm, "Tamanho", 0.0);
        let font_size = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(58.0, 26.0)),
        );
        font_size.setStringValue(ns_string!("12"));
        font_size.setAccessibilityLabel(Some(ns_string!("Tamanho da fonte em pontos")));
        font_size.setAccessibilityIdentifier(Some(ns_string!("indicator.font.size")));
        let points_label = text_label(mtm, "pt", 0.0);
        points_label.setFrameOrigin(NSPoint::new(166.0, 0.0));

        let weight_label = text_label(mtm, "Peso", 0.0);
        let font_weight = segmented(
            mtm,
            &["Regular", "Médio", "Negrito"],
            "indicator.font.weight",
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(300.0, 28.0)),
            placeholder_target,
            None,
        );
        let reset_typography = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Restaurar tipografia"),
                None,
                None,
                mtm,
            )
        };
        reset_typography.setFrame(NSRect::new(
            NSPoint::new(400.0, 0.0),
            NSSize::new(150.0, 28.0),
        ));
        let font_fallback_warning = warning_label(mtm, 0.0);
        let layout_warning = warning_label(mtm, 0.0);

        let update_heading = heading(mtm, "Atualização", 0.0);
        let interval_label = text_label(mtm, "Intervalo", 0.0);
        let interval_field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(58.0, 26.0)),
        );
        interval_field.setStringValue(ns_string!("2"));
        interval_field.setAccessibilityLabel(Some(ns_string!("Intervalo de atualização")));
        interval_field.setAccessibilityIdentifier(Some(ns_string!("indicator.refresh.interval")));
        let interval_stepper = NSStepper::initWithFrame(
            NSStepper::alloc(mtm),
            NSRect::new(NSPoint::new(164.0, 0.0), NSSize::new(20.0, 28.0)),
        );
        interval_stepper.setMinValue(1.0);
        interval_stepper.setMaxValue(60.0);
        interval_stepper.setIncrement(1.0);
        interval_stepper.setValueWraps(false);
        interval_stepper
            .setAccessibilityLabel(Some(ns_string!("Ajustar intervalo de atualização")));
        let seconds_label = text_label(mtm, "segundos", 0.0);
        seconds_label.setFrameOrigin(NSPoint::new(194.0, 0.0));
        let interval_help = text_label(
            mtm,
            "Intervalos menores atualizam com mais frequência e usam mais recursos.",
            0.0,
        );
        interval_help.setFrameSize(NSSize::new(500.0, 24.0));
        interval_help.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let interval_error = warning_label(mtm, 0.0);
        let reset_refresh_interval = reset_button(
            mtm,
            "Restaurar atualização",
            NSRect::new(NSPoint::new(350.0, 0.0), NSSize::new(190.0, 28.0)),
        );

        let defaults = IndicatorPreferences::default();
        let target = IndicatorControlsTarget::new(
            mtm,
            IndicatorControlsTargetIvars {
                proxy: proxy.clone(),
                applying: Cell::new(false),
                selected_family: RefCell::new(defaults.typography.family.clone()),
                selected_font_size: Cell::new(defaults.typography.size),
                interval_draft: RefCell::new(IntervalDraft::new(defaults.refresh_interval)),
                font_resources: RefCell::new(None),
                font_size: font_size.clone(),
                interval_field: interval_field.clone(),
                interval_stepper: interval_stepper.clone(),
                interval_error: interval_error.clone(),
            },
        );
        configure_actions(
            &target,
            &cpu_mode,
            &reset_cpu_and_ram,
            &ram_mode,
            &labels_visible,
            &labels_mode,
            &reset_labels,
            &font_family,
            &font_size,
            &font_weight,
            &reset_typography,
            &interval_field,
            &interval_stepper,
            &reset_refresh_interval,
        );

        for child in [
            &*colors_heading as &NSView,
            &*cpu_label,
            &*cpu_mode,
            &*reset_cpu_and_ram,
            cpu_editor.view(),
            &*ram_label,
            &*ram_mode,
            ram_editor.view(),
            &*labels_heading,
            &*labels_visible,
            &*labels_mode,
            &*reset_labels,
            labels_editor.view(),
            &*typography_heading,
            &*family_label,
            &*font_family,
            &*size_label,
            &*font_size,
            &*points_label,
            &*weight_label,
            &*font_weight,
            &*reset_typography,
            &*font_fallback_warning,
            &*layout_warning,
            &*update_heading,
            &*interval_label,
            &*interval_field,
            &*interval_stepper,
            &*reset_refresh_interval,
            &*seconds_label,
            &*interval_help,
            &*interval_error,
        ] {
            view.addSubview(child);
        }

        let layout_views = IndicatorLayoutViews {
            colors_heading: colors_heading.clone(),
            reset_cpu_and_ram: reset_cpu_and_ram.clone(),
            cpu_label: cpu_label.clone(),
            cpu_mode: cpu_mode.clone(),
            cpu_editor: cpu_editor.view().retain(),
            ram_label: ram_label.clone(),
            ram_mode: ram_mode.clone(),
            ram_editor: ram_editor.view().retain(),
            labels_heading: labels_heading.clone(),
            labels_visible: labels_visible.clone(),
            labels_mode: labels_mode.clone(),
            reset_labels: reset_labels.clone(),
            labels_editor: labels_editor.view().retain(),
            typography_heading: typography_heading.clone(),
            family_label: family_label.clone(),
            font_family: font_family.clone(),
            size_label: size_label.clone(),
            font_size: font_size.clone(),
            points_label: points_label.clone(),
            weight_label: weight_label.clone(),
            font_weight: font_weight.clone(),
            reset_typography: reset_typography.clone(),
            font_fallback_warning: font_fallback_warning.clone(),
            layout_warning: layout_warning.clone(),
            update_heading: update_heading.clone(),
            interval_label: interval_label.clone(),
            interval_field: interval_field.clone(),
            interval_stepper: interval_stepper.clone(),
            seconds_label: seconds_label.clone(),
            reset_refresh_interval: reset_refresh_interval.clone(),
            interval_help: interval_help.clone(),
            interval_error: interval_error.clone(),
        };
        let controls = Self {
            view,
            layout_views,
            content_height: Cell::new(0.0),
            cpu_mode,
            reset_cpu_and_ram,
            ram_mode,
            labels_visible,
            labels_mode,
            reset_labels,
            font_family,
            font_size,
            font_weight,
            reset_typography,
            interval_field,
            interval_stepper,
            reset_refresh_interval,
            cpu_editor,
            ram_editor,
            labels_editor,
            target,
        };
        controls.apply(&IndicatorPreferences::default());
        controls
    }

    pub(super) fn apply(&self, preferences: &IndicatorPreferences) {
        self.target.ivars().applying.set(true);
        self.cpu_mode
            .setSelectedSegment(match preferences.cpu_color.mode {
                MetricColorMode::Dynamic => 0,
                MetricColorMode::Fixed => 1,
            });
        self.ram_mode
            .setSelectedSegment(match preferences.ram_color.mode {
                MetricColorMode::Dynamic => 0,
                MetricColorMode::Fixed => 1,
            });
        self.labels_visible.setState(if preferences.labels.visible {
            NSControlStateValueOn
        } else {
            0
        });
        self.labels_mode
            .setSelectedSegment(match preferences.labels.color_mode {
                LabelColorMode::Neutral => 0,
                LabelColorMode::MatchMetric => 1,
                LabelColorMode::Fixed => 2,
            });
        self.target
            .ivars()
            .selected_family
            .replace(preferences.typography.family.clone());
        self.target
            .ivars()
            .selected_font_size
            .set(preferences.typography.size);
        self.target
            .ivars()
            .interval_draft
            .borrow_mut()
            .sync(preferences.refresh_interval);
        self.apply_typography(&preferences.typography);
        let interval = self.target.ivars().interval_draft.borrow();
        self.interval_field
            .setStringValue(&objc2_foundation::NSString::from_str(interval.text()));
        self.interval_stepper
            .setIntegerValue(interval.valid_interval().seconds().into());
        set_inline_error(
            &self.layout_views.interval_error,
            interval.error().map(|error| error.message()),
        );
        drop(interval);

        let cpu_state = ColorEditorState::from_preferences(preferences.cpu_color.fixed);
        let ram_state = ColorEditorState::from_preferences(preferences.ram_color.fixed);
        let labels_state = ColorEditorState::from_preferences(preferences.labels.fixed);
        self.cpu_editor.apply(&cpu_state);
        self.ram_editor.apply(&ram_state);
        self.labels_editor.apply(&labels_state);

        let cpu_fixed = preferences.cpu_color.mode == MetricColorMode::Fixed;
        let ram_fixed = preferences.ram_color.mode == MetricColorMode::Fixed;
        let labels_fixed = preferences.labels.color_mode == LabelColorMode::Fixed;
        self.cpu_editor.view().setHidden(!cpu_fixed);
        self.ram_editor.view().setHidden(!ram_fixed);
        self.labels_editor.view().setHidden(!labels_fixed);
        if !cpu_fixed {
            self.cpu_editor.deactivate();
        }
        if !ram_fixed {
            self.ram_editor.deactivate();
        }
        if !labels_fixed {
            self.labels_editor.deactivate();
        }
        self.apply_layout(IndicatorControlsVisibility {
            cpu_editor: cpu_fixed,
            ram_editor: ram_fixed,
            labels_editor: labels_fixed,
        });

        unsafe {
            if cpu_fixed {
                self.cpu_editor
                    .configure_key_order(&self.cpu_mode, &self.ram_mode, &cpu_state);
            } else {
                self.cpu_mode.setNextKeyView(Some(&self.ram_mode));
            }
            if ram_fixed {
                self.ram_editor.configure_key_order(
                    &self.ram_mode,
                    &self.reset_cpu_and_ram,
                    &ram_state,
                );
            } else {
                self.ram_mode.setNextKeyView(Some(&self.reset_cpu_and_ram));
            }
            self.reset_cpu_and_ram
                .setNextKeyView(Some(&self.labels_visible));
            self.labels_visible.setNextKeyView(Some(&self.labels_mode));
            if labels_fixed {
                self.labels_editor.configure_key_order(
                    &self.labels_mode,
                    &self.reset_labels,
                    &labels_state,
                );
            } else {
                self.labels_mode.setNextKeyView(Some(&self.reset_labels));
            }
            self.reset_labels.setNextKeyView(Some(&self.font_family));
            self.font_family.setNextKeyView(Some(&self.font_size));
            self.font_size.setNextKeyView(Some(&self.font_weight));
            self.font_weight
                .setNextKeyView(Some(&self.reset_typography));
            self.reset_typography
                .setNextKeyView(Some(&self.interval_field));
            self.interval_field
                .setNextKeyView(Some(&self.interval_stepper));
            self.interval_stepper
                .setNextKeyView(Some(&self.reset_refresh_interval));
            self.reset_refresh_interval
                .setNextKeyView(Some(&self.cpu_mode));
        }
        self.target.ivars().applying.set(false);
    }

    fn apply_layout(&self, visibility: IndicatorControlsVisibility) {
        let layout = IndicatorControlsLayout::new(visibility);
        self.layout_views.apply(&layout);
        self.view
            .setFrameSize(NSSize::new(560.0, layout.content_height()));
        self.content_height.set(layout.content_height());
    }

    pub(super) fn content_height(&self) -> f64 {
        self.content_height.get()
    }

    pub(super) fn wells(&self) -> Vec<Retained<NSColorWell>> {
        self.cpu_editor
            .wells()
            .into_iter()
            .chain(self.ram_editor.wells())
            .chain(self.labels_editor.wells())
            .collect()
    }

    pub(super) fn view(&self) -> &NSStackView {
        &self.view
    }

    pub(super) fn first_key_view(&self) -> &NSSegmentedControl {
        &self.cpu_mode
    }

    pub(super) fn last_key_view(&self) -> &NSButton {
        &self.reset_refresh_interval
    }

    pub(super) fn apply_diagnostics(
        &self,
        fallback: Option<&IndicatorFontFallback>,
        layout: &IndicatorLayoutDiagnostics,
    ) {
        let fallback_message = fallback.map(|fallback| {
            let requested = family_name(&fallback.requested_family);
            format!(
                "A fonte {requested} não está disponível; usando {} sem alterar sua escolha.",
                fallback.resolved_family
            )
        });
        set_warning_text(
            &self.layout_views.font_fallback_warning,
            fallback_message.as_deref(),
        );

        let diagnostics = [layout.status, Some(layout.light), Some(layout.dark)]
            .into_iter()
            .flatten();
        let (too_tall, too_wide) = diagnostics.fold((false, false), |state, item| {
            (
                state.0 || item.exceeds_menu_bar_height,
                state.1 || item.exceeds_curated_width,
            )
        });
        let message = match (too_tall, too_wide) {
            (true, true) => {
                Some("Esta tipografia pode cortar as linhas e ocupar largura excessiva.")
            }
            (true, false) => Some("Esta tipografia pode cortar as linhas na altura da menu bar."),
            (false, true) => Some("Esta tipografia pode ocupar largura excessiva na menu bar."),
            (false, false) => None,
        };
        set_warning_text(&self.layout_views.layout_warning, message);
    }

    fn apply_typography(&self, typography: &TypographyPreferences) {
        let family = family_name(&typography.family);
        self.font_family
            .setTitle(&objc2_foundation::NSString::from_str(&family));
        self.font_family
            .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
                "Família da fonte, {family}"
            ))));
        self.font_size
            .setStringValue(&objc2_foundation::NSString::from_str(
                &typography.size.points().to_string(),
            ));
        self.font_weight
            .setSelectedSegment(match typography.weight {
                FontWeight::Regular => 0,
                FontWeight::Medium => 1,
                FontWeight::Bold => 2,
            });
    }
}

fn segmented(
    mtm: MainThreadMarker,
    labels: &[&str],
    identifier: &str,
    frame: NSRect,
    target: Option<&IndicatorControlsTarget>,
    action: Option<objc2::runtime::Sel>,
) -> Retained<NSSegmentedControl> {
    let labels = labels
        .iter()
        .map(|label| objc2_foundation::NSString::from_str(label))
        .collect::<Vec<_>>();
    let refs = labels.iter().map(|label| &**label).collect::<Vec<_>>();
    let control = unsafe {
        NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
            &NSArray::from_slice(&refs),
            NSSegmentSwitchTracking::SelectOne,
            target.map(|target| target as &AnyObject),
            action,
            mtm,
        )
    };
    control.setFrame(frame);
    control.setSelectedSegment(0);
    control.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(identifier)));
    control
}

fn heading(mtm: MainThreadMarker, title: &str, y: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(title), mtm);
    field.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(92.0, 24.0)));
    field.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    field
}

fn text_label(mtm: MainThreadMarker, title: &str, y: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(title), mtm);
    field.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(92.0, 24.0)));
    field
}

fn warning_label(mtm: MainThreadMarker, y: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(ns_string!(""), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(100.0, y),
        NSSize::new(450.0, 24.0),
    ));
    field.setTextColor(Some(&NSColor::systemOrangeColor()));
    field.setHidden(true);
    field
}

fn reset_button(mtm: MainThreadMarker, title: &str, frame: NSRect) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &objc2_foundation::NSString::from_str(title),
            None,
            None,
            mtm,
        )
    };
    button.setFrame(frame);
    button
}

#[allow(clippy::too_many_arguments)]
fn configure_actions(
    target: &IndicatorControlsTarget,
    cpu_mode: &NSSegmentedControl,
    reset_cpu_and_ram: &NSButton,
    ram_mode: &NSSegmentedControl,
    labels_visible: &NSButton,
    labels_mode: &NSSegmentedControl,
    reset_labels: &NSButton,
    font_family: &NSButton,
    font_size: &NSTextField,
    font_weight: &NSSegmentedControl,
    reset_typography: &NSButton,
    interval_field: &NSTextField,
    interval_stepper: &NSStepper,
    reset_refresh_interval: &NSButton,
) {
    unsafe {
        for (control, action) in [
            (
                &**cpu_mode as &objc2_app_kit::NSControl,
                sel!(changeCpuColorMode:),
            ),
            (
                &**reset_cpu_and_ram as &objc2_app_kit::NSControl,
                sel!(resetCpuAndRam:),
            ),
            (
                &**ram_mode as &objc2_app_kit::NSControl,
                sel!(changeRamColorMode:),
            ),
            (
                &**labels_visible as &objc2_app_kit::NSControl,
                sel!(toggleLabelsVisible:),
            ),
            (
                &**labels_mode as &objc2_app_kit::NSControl,
                sel!(changeLabelColorMode:),
            ),
            (
                &**reset_labels as &objc2_app_kit::NSControl,
                sel!(resetLabels:),
            ),
            (
                &**font_family as &objc2_app_kit::NSControl,
                sel!(openFontPicker:),
            ),
            (
                &**font_size as &objc2_app_kit::NSControl,
                sel!(commitFontSize:),
            ),
            (
                &**font_weight as &objc2_app_kit::NSControl,
                sel!(changeFontWeight:),
            ),
            (
                &**reset_typography as &objc2_app_kit::NSControl,
                sel!(resetTypography:),
            ),
            (
                &**interval_field as &objc2_app_kit::NSControl,
                sel!(commitRefreshInterval:),
            ),
            (
                &**interval_stepper as &objc2_app_kit::NSControl,
                sel!(stepRefreshInterval:),
            ),
            (
                &**reset_refresh_interval as &objc2_app_kit::NSControl,
                sel!(resetRefreshInterval:),
            ),
        ] {
            control.setTarget(Some(target as &AnyObject));
            control.setAction(Some(action));
        }
        font_size.setDelegate(Some(ProtocolObject::from_ref(target)));
        interval_field.setDelegate(Some(ProtocolObject::from_ref(target)));
    }
}

fn family_name(family: &FontFamilyPreference) -> String {
    match family {
        FontFamilyPreference::SystemMonospaced => "System Monospaced".to_owned(),
        FontFamilyPreference::Named(family) => family.clone(),
    }
}

fn notification_text_field(notification: &NSNotification) -> Option<Retained<NSTextField>> {
    notification.object()?.downcast::<NSTextField>().ok()
}

fn set_inline_error(field: &NSTextField, message: Option<&str>) {
    set_warning_text(field, message);
    field.setAccessibilityLabel(message.map(objc2_foundation::NSString::from_str).as_deref());
}

fn set_warning_text(field: &NSTextField, message: Option<&str>) {
    field.setStringValue(&objc2_foundation::NSString::from_str(message.unwrap_or("")));
    field.setHidden(message.is_none());
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::get_or_create_font_resources;

    #[test]
    fn font_picker_resources_are_created_only_on_first_request() {
        let creations = Cell::new(0);
        let mut resources = None;
        assert!(resources.is_none());

        let first = *get_or_create_font_resources(&mut resources, || {
            creations.set(creations.get() + 1);
            7
        });
        let second = *get_or_create_font_resources(&mut resources, || {
            creations.set(creations.get() + 1);
            8
        });

        assert_eq!((first, second), (7, 7));
        assert_eq!(creations.get(), 1);
    }
}
