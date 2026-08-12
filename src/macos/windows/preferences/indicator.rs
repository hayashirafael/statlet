use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColorWell, NSControlStateValueOn, NSFont, NSSegmentSwitchTracking,
    NSSegmentedControl, NSStackView, NSTextField, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};
use statlet::core::{AppEvent, IndicatorPreferenceChange};
use statlet::indicator_preferences::{
    IndicatorPreferences, LabelColorMode, MetricColorMode, MetricKind,
};
use statlet::preferences_view::ColorEditorState;
use tao::event_loop::EventLoopProxy;

use super::color_editor::{ColorBinding, ColorEditor};
use crate::macos::RuntimeEvent;

struct IndicatorControlsTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    applying: Cell<bool>,
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
    }
);

impl IndicatorControlsTarget {
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(IndicatorControlsTargetIvars {
            proxy,
            applying: Cell::new(false),
        });
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
        let _ = self
            .ivars()
            .proxy
            .send_event(RuntimeEvent::App(AppEvent::UpdateIndicator(change)));
    }
}

pub(super) struct IndicatorControls {
    view: Retained<NSStackView>,
    cpu_mode: Retained<NSSegmentedControl>,
    ram_mode: Retained<NSSegmentedControl>,
    labels_visible: Retained<NSButton>,
    labels_mode: Retained<NSSegmentedControl>,
    cpu_editor: ColorEditor,
    ram_editor: ColorEditor,
    labels_editor: ColorEditor,
    target: Retained<IndicatorControlsTarget>,
}

impl IndicatorControls {
    pub(super) fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let view = NSStackView::initWithFrame(
            NSStackView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 820.0)),
        );
        let target = IndicatorControlsTarget::new(mtm, proxy.clone());

        let cpu_heading = heading(mtm, "CPU", 790.0);
        let cpu_mode = segmented(
            mtm,
            &["Dinâmica", "Fixa"],
            "indicator.cpu.mode",
            NSRect::new(NSPoint::new(100.0, 756.0), NSSize::new(240.0, 28.0)),
            &target,
            sel!(changeCpuColorMode:),
        );
        let cpu_editor = ColorEditor::new(
            mtm,
            ColorBinding::MetricShared(MetricKind::Cpu),
            proxy.clone(),
        );
        cpu_editor.view().setFrame(NSRect::new(
            NSPoint::new(0.0, 588.0),
            NSSize::new(540.0, 160.0),
        ));

        let ram_heading = heading(mtm, "RAM", 558.0);
        let ram_mode = segmented(
            mtm,
            &["Dinâmica", "Fixa"],
            "indicator.ram.mode",
            NSRect::new(NSPoint::new(100.0, 524.0), NSSize::new(240.0, 28.0)),
            &target,
            sel!(changeRamColorMode:),
        );
        let ram_editor = ColorEditor::new(
            mtm,
            ColorBinding::MetricShared(MetricKind::Ram),
            proxy.clone(),
        );
        ram_editor.view().setFrame(NSRect::new(
            NSPoint::new(0.0, 356.0),
            NSSize::new(540.0, 160.0),
        ));

        let labels_heading = heading(mtm, "Rótulos", 326.0);
        let labels_visible = unsafe {
            NSButton::checkboxWithTitle_target_action(
                ns_string!("Mostrar rótulos C/R"),
                Some(&*target as &AnyObject),
                Some(sel!(toggleLabelsVisible:)),
                mtm,
            )
        };
        labels_visible.setFrame(NSRect::new(
            NSPoint::new(0.0, 292.0),
            NSSize::new(220.0, 24.0),
        ));
        labels_visible.setAccessibilityIdentifier(Some(ns_string!("indicator.labels.visible")));
        let labels_mode = segmented(
            mtm,
            &["Neutra", "Igual ao valor", "Personalizada"],
            "indicator.labels.mode",
            NSRect::new(NSPoint::new(100.0, 254.0), NSSize::new(390.0, 28.0)),
            &target,
            sel!(changeLabelColorMode:),
        );
        let labels_editor = ColorEditor::new(mtm, ColorBinding::LabelShared, proxy);
        labels_editor.view().setFrame(NSRect::new(
            NSPoint::new(0.0, 86.0),
            NSSize::new(540.0, 160.0),
        ));

        for child in [
            &*cpu_heading as &NSView,
            &*cpu_mode,
            cpu_editor.view(),
            &*ram_heading,
            &*ram_mode,
            ram_editor.view(),
            &*labels_heading,
            &*labels_visible,
            &*labels_mode,
            labels_editor.view(),
        ] {
            view.addSubview(child);
        }

        let controls = Self {
            view,
            cpu_mode,
            ram_mode,
            labels_visible,
            labels_mode,
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
                    &self.labels_visible,
                    &ram_state,
                );
            } else {
                self.ram_mode.setNextKeyView(Some(&self.labels_visible));
            }
            self.labels_visible.setNextKeyView(Some(&self.labels_mode));
            if labels_fixed {
                self.labels_editor.configure_key_order(
                    &self.labels_mode,
                    &self.cpu_mode,
                    &labels_state,
                );
            } else {
                self.labels_mode.setNextKeyView(Some(&self.cpu_mode));
            }
        }
        self.target.ivars().applying.set(false);
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
}

fn segmented(
    mtm: MainThreadMarker,
    labels: &[&str],
    identifier: &str,
    frame: NSRect,
    target: &IndicatorControlsTarget,
    action: objc2::runtime::Sel,
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
            Some(target as &AnyObject),
            Some(action),
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
