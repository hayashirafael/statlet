use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly, Message};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColor, NSColorWell, NSControl, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSFont, NSImage, NSImageView, NSModalResponseOK, NSOpenPanel,
    NSPopUpButton, NSSegmentSwitchTracking, NSSegmentedControl, NSSlider, NSStackView, NSStepper,
    NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSNotification, NSNumber, NSNumberFormatter,
    NSNumberFormatterStyle, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use statlet::core::{AppEvent, IndicatorPreferenceChange};
use statlet::icon_assets::IconAssetStore;
use statlet::indicator_preferences::{
    FontFamilyPreference, FontSize, FontWeight, IndicatorLabel, IndicatorPreferenceGroup,
    IndicatorPreferences, LabelColorMode, LabelSpacing, MetricColorMode, MetricIdentifierMode,
    MetricKind, SystemSymbolName, SystemSymbolSize, TypographyPreferences,
};
use statlet::preferences_view::{
    ColorEditorState, IdentifierDetailPresentation, IdentifierEditingFocusTarget,
    IdentifierEditingPresentation, IndicatorControlsLayout, IndicatorControlsVisibility,
    IntervalDraft, IntervalFieldFormat, LabelEditingFocusTarget, LabelEditingPresentation,
    MetricIdentifierControlPresentation, PreferencesArea, TypographyWarningKind,
};
use statlet::runtime_profile::RuntimePresentation;
use tao::event_loop::EventLoopProxy;

use super::color_editor::{ColorBinding, ColorEditor};
use super::font_picker::FontPicker;
use super::{
    configure_discrete_slider, font_size_slider_contract, label_spacing_slider_contract,
    system_symbol_size_slider_contract, IndicatorFontFallback, IndicatorLayoutDiagnostics,
};
use crate::macos::fonts::FontCatalog;
use crate::macos::RuntimeEvent;

struct IndicatorControlsTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    presentation: RuntimePresentation,
    applying: Cell<bool>,
    selected_family: RefCell<FontFamilyPreference>,
    selected_font_size: Cell<FontSize>,
    interval_draft: RefCell<IntervalDraft>,
    font_resources: RefCell<Option<FontResources>>,
    selected_cpu_label: RefCell<IndicatorLabel>,
    selected_ram_label: RefCell<IndicatorLabel>,
    selected_label_spacing: Cell<LabelSpacing>,
    selected_cpu_identifier_mode: Cell<MetricIdentifierMode>,
    selected_ram_identifier_mode: Cell<MetricIdentifierMode>,
    selected_system_symbol_size: Cell<SystemSymbolSize>,
    cpu_png_available: Cell<bool>,
    ram_png_available: Cell<bool>,
    system_symbol_size: Retained<NSSlider>,
    system_symbol_size_value: Retained<NSTextField>,
    cpu_label_field: Retained<NSTextField>,
    ram_label_field: Retained<NSTextField>,
    label_spacing: Retained<NSSlider>,
    label_spacing_value: Retained<NSTextField>,
    font_size: Retained<NSSlider>,
    font_size_value: Retained<NSTextField>,
    interval_field: Retained<NSTextField>,
    interval_stepper: Retained<NSStepper>,
    interval_error: Retained<NSTextField>,
}

struct FontResources {
    picker: FontPicker,
    catalog: FontCatalog,
}

#[derive(Debug, Eq, PartialEq)]
struct PngPanelPresentation {
    title: Option<String>,
    prompt: String,
    message: String,
}

fn png_panel_presentation(
    metric: MetricKind,
    presentation: &RuntimePresentation,
) -> PngPanelPresentation {
    const PROMPT: &str = "Escolher PNG";
    const MESSAGE: &str =
        "Escolha um PNG com transparência. O Statlet reduzirá e otimizará uma cópia.";
    let production_title = match metric {
        MetricKind::Cpu => "Escolher PNG da CPU",
        MetricKind::Ram => "Escolher PNG da RAM",
    };
    PngPanelPresentation {
        title: presentation
            .dev_marker()
            .map(|_| presentation.window_title(production_title)),
        prompt: PROMPT.to_owned(),
        message: presentation.status_metadata(MESSAGE),
    }
}

fn get_or_create_font_resources<T>(slot: &mut Option<T>, create: impl FnOnce() -> T) -> &mut T {
    slot.get_or_insert_with(create)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IndicatorAreaVisibility {
    colors: bool,
    labels: bool,
    typography: bool,
    refresh: bool,
}

impl IndicatorAreaVisibility {
    const fn new(colors: bool, labels: bool, typography: bool, refresh: bool) -> Self {
        Self {
            colors,
            labels,
            typography,
            refresh,
        }
    }

    pub(super) const fn for_area(area: PreferencesArea) -> Self {
        match area {
            PreferencesArea::Colors => Self::new(true, false, false, false),
            PreferencesArea::Labels => Self::new(false, true, false, false),
            PreferencesArea::Typography => Self::new(false, false, true, false),
            PreferencesArea::Refresh => Self::new(false, false, false, true),
            PreferencesArea::DiskAndMole => Self::new(false, false, false, false),
        }
    }
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

        #[unsafe(method(resetIdentifiers:))]
        fn reset_identifiers(&self, _sender: &NSButton) {
            self.reset_group(IndicatorPreferenceGroup::Identifiers);
        }

        #[unsafe(method(changeRamColorMode:))]
        fn change_ram_color_mode(&self, sender: &NSSegmentedControl) {
            self.send_metric_mode(MetricKind::Ram, sender);
        }

        #[unsafe(method(changeCpuIdentifierMode:))]
        fn change_cpu_identifier_mode(&self, sender: &NSSegmentedControl) {
            self.change_identifier_mode(MetricKind::Cpu, sender);
        }

        #[unsafe(method(changeRamIdentifierMode:))]
        fn change_ram_identifier_mode(&self, sender: &NSSegmentedControl) {
            self.change_identifier_mode(MetricKind::Ram, sender);
        }

        #[unsafe(method(changeCpuSystemSymbol:))]
        fn change_cpu_system_symbol(&self, sender: &NSPopUpButton) {
            self.change_system_symbol(MetricKind::Cpu, sender);
        }

        #[unsafe(method(changeRamSystemSymbol:))]
        fn change_ram_system_symbol(&self, sender: &NSPopUpButton) {
            self.change_system_symbol(MetricKind::Ram, sender);
        }

        #[unsafe(method(changeSystemSymbolSize:))]
        fn change_system_symbol_size(&self, sender: &NSSlider) {
            self.apply_system_symbol_size(sender.integerValue());
        }

        #[unsafe(method(chooseCpuPng:))]
        fn choose_cpu_png(&self, sender: &NSButton) {
            self.present_png_panel(MetricKind::Cpu, sender.window().as_deref(), None);
        }

        #[unsafe(method(chooseRamPng:))]
        fn choose_ram_png(&self, sender: &NSButton) {
            self.present_png_panel(MetricKind::Ram, sender.window().as_deref(), None);
        }

        #[unsafe(method(removeCpuPng:))]
        fn remove_cpu_png(&self, _sender: &NSButton) {
            self.send_event(AppEvent::RemoveMetricPng(MetricKind::Cpu));
        }

        #[unsafe(method(removeRamPng:))]
        fn remove_ram_png(&self, _sender: &NSButton) {
            self.send_event(AppEvent::RemoveMetricPng(MetricKind::Ram));
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

        #[unsafe(method(commitCpuLabel:))]
        fn commit_cpu_label_action(&self, sender: &NSTextField) {
            self.commit_label(sender, MetricKind::Cpu, true);
        }

        #[unsafe(method(commitRamLabel:))]
        fn commit_ram_label_action(&self, sender: &NSTextField) {
            self.commit_label(sender, MetricKind::Ram, true);
        }

        #[unsafe(method(changeLabelSpacing:))]
        fn change_label_spacing(&self, sender: &NSSlider) {
            self.apply_label_spacing(sender.integerValue());
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
                picker: FontPicker::new(marker, proxy, self.ivars().presentation.clone()),
                catalog: FontCatalog::new(marker),
            });
            resources.catalog.refresh();
            resources.picker.refresh_catalog(&resources.catalog);
            resources
                .picker
                .present(&parent, &resources.catalog, &selected);
        }

        #[unsafe(method(changeFontSize:))]
        fn change_font_size(&self, sender: &NSSlider) {
            self.apply_font_size(sender.integerValue());
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
            if std::ptr::eq(&*field, &*self.ivars().interval_field) {
                self.commit_refresh_interval(&field);
            } else if std::ptr::eq(&*field, &*self.ivars().cpu_label_field) {
                self.commit_label(&field, MetricKind::Cpu, true);
            } else if std::ptr::eq(&*field, &*self.ivars().ram_label_field) {
                self.commit_label(&field, MetricKind::Ram, true);
            }
        }

        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            if self.ivars().applying.get() {
                return;
            }
            let Some(field) = notification_text_field(notification) else {
                return;
            };
            if std::ptr::eq(&*field, &*self.ivars().cpu_label_field) {
                self.commit_label(&field, MetricKind::Cpu, false);
            } else if std::ptr::eq(&*field, &*self.ivars().ram_label_field) {
                self.commit_label(&field, MetricKind::Ram, false);
            }
        }

        #[unsafe(method(control:didFailToFormatString:errorDescription:))]
        fn control_did_fail_to_format_string(
            &self,
            control: &NSControl,
            string: &NSString,
            _error: Option<&NSString>,
        ) -> bool {
            if std::ptr::eq(control, &**self.ivars().interval_field) {
                self.commit_refresh_interval_text(&string.to_string());
                IntervalFieldFormat::seconds().accepts_invalid_commit_for_domain_validation()
            } else {
                false
            }
        }

        #[unsafe(method(control:didFailToValidatePartialString:errorDescription:))]
        fn control_did_fail_to_validate_partial_string(
            &self,
            control: &NSControl,
            string: &NSString,
            _error: Option<&NSString>,
        ) {
            if std::ptr::eq(control, &**self.ivars().interval_field) {
                self.commit_refresh_interval_text(&string.to_string());
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

    fn change_identifier_mode(&self, metric: MetricKind, sender: &NSSegmentedControl) {
        if self.ivars().applying.get() {
            return;
        }
        let mode = match sender.selectedSegment() {
            1 => MetricIdentifierMode::SystemSymbol,
            2 => MetricIdentifierMode::Png,
            _ => MetricIdentifierMode::Text,
        };
        let png_available = match metric {
            MetricKind::Cpu => self.ivars().cpu_png_available.get(),
            MetricKind::Ram => self.ivars().ram_png_available.get(),
        };
        if mode == MetricIdentifierMode::Png && !png_available {
            self.present_png_panel(metric, sender.window().as_deref(), Some(sender.retain()));
            return;
        }
        self.send(IndicatorPreferenceChange::SetMetricIdentifierMode { metric, mode });
    }

    fn change_system_symbol(&self, metric: MetricKind, sender: &NSPopUpButton) {
        if self.ivars().applying.get() {
            return;
        }
        let Ok(index) = usize::try_from(sender.indexOfSelectedItem()) else {
            return;
        };
        let Some(option) = SystemSymbolName::curated_options().get(index) else {
            return;
        };
        let Ok(symbol) = SystemSymbolName::new(option.name()) else {
            return;
        };
        self.send(IndicatorPreferenceChange::SetMetricSystemSymbol { metric, symbol });
    }

    fn present_png_panel(
        &self,
        metric: MetricKind,
        parent: Option<&objc2_app_kit::NSWindow>,
        restore_mode: Option<Retained<NSSegmentedControl>>,
    ) {
        let Some(parent) = parent else {
            if let Some(control) = restore_mode {
                restore_identifier_segment(&control, self.selected_identifier_mode(metric));
            }
            return;
        };
        let marker = MainThreadMarker::new().expect("PNG picker actions run on main thread");
        let panel = NSOpenPanel::openPanel(marker);
        let panel_presentation = png_panel_presentation(metric, &self.ivars().presentation);
        panel.setCanChooseDirectories(false);
        panel.setCanChooseFiles(true);
        panel.setAllowsMultipleSelection(false);
        panel.setResolvesAliases(true);
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[ns_string!("png")])));
        if let Some(title) = panel_presentation.title.as_deref() {
            panel.setTitle(Some(&NSString::from_str(title)));
        }
        panel.setPrompt(Some(&NSString::from_str(&panel_presentation.prompt)));
        panel.setMessage(Some(&NSString::from_str(&panel_presentation.message)));

        let proxy = self.ivars().proxy.clone();
        let panel_for_completion = panel.clone();
        let previous_mode = self.selected_identifier_mode(metric);
        let completion = RcBlock::new(move |result| {
            if result == NSModalResponseOK {
                if let Some(path) = panel_for_completion
                    .URL()
                    .and_then(|url| url.path())
                    .map(|path| PathBuf::from(path.to_string()))
                {
                    let _ = proxy.send_event(RuntimeEvent::App(AppEvent::ChooseMetricPng {
                        metric,
                        source: path,
                    }));
                    return;
                }
            }
            if let Some(control) = &restore_mode {
                restore_identifier_segment(control, previous_mode);
            }
            let _ = proxy.send_event(RuntimeEvent::App(AppEvent::CancelMetricPngImport(metric)));
        });
        panel.beginSheetModalForWindow_completionHandler(parent, &completion);
    }

    fn selected_identifier_mode(&self, metric: MetricKind) -> MetricIdentifierMode {
        match metric {
            MetricKind::Cpu => self.ivars().selected_cpu_identifier_mode.get(),
            MetricKind::Ram => self.ivars().selected_ram_identifier_mode.get(),
        }
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

    fn apply_font_size(&self, value: isize) {
        if self.ivars().applying.get() {
            return;
        }
        let size = u8::try_from(value)
            .ok()
            .and_then(|points| FontSize::try_from(points).ok());
        let Some(size) = size else {
            self.ivars()
                .font_size
                .setIntegerValue(self.ivars().selected_font_size.get().points().into());
            return;
        };
        set_slider_value_text(
            &self.ivars().font_size,
            &self.ivars().font_size_value,
            &format!("{} pt", size.points()),
        );
        if size != self.ivars().selected_font_size.replace(size) {
            self.send(IndicatorPreferenceChange::SetFontSize(size));
        }
    }

    fn apply_system_symbol_size(&self, value: isize) {
        if self.ivars().applying.get() {
            return;
        }
        let size = u8::try_from(value)
            .ok()
            .and_then(|points| SystemSymbolSize::try_from(points).ok());
        let Some(size) = size else {
            self.ivars().system_symbol_size.setIntegerValue(
                self.ivars()
                    .selected_system_symbol_size
                    .get()
                    .points()
                    .into(),
            );
            return;
        };
        set_slider_value_text(
            &self.ivars().system_symbol_size,
            &self.ivars().system_symbol_size_value,
            &format!("{} pt", size.points()),
        );
        if size != self.ivars().selected_system_symbol_size.replace(size) {
            self.send(IndicatorPreferenceChange::SetSystemSymbolSize(size));
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
            Err(error) => {
                let message = error.message();
                set_inline_error(&self.ivars().interval_error, Some(&message));
            }
        }
    }

    fn commit_label(&self, field: &NSTextField, metric: MetricKind, restore_invalid: bool) {
        if self.ivars().applying.get() {
            return;
        }
        let Ok(label) = IndicatorLabel::new(field.stringValue().to_string()) else {
            if restore_invalid {
                let selected = match metric {
                    MetricKind::Cpu => self.ivars().selected_cpu_label.borrow().clone(),
                    MetricKind::Ram => self.ivars().selected_ram_label.borrow().clone(),
                };
                field.setStringValue(&objc2_foundation::NSString::from_str(selected.as_str()));
            }
            return;
        };
        if restore_invalid {
            field.setStringValue(&objc2_foundation::NSString::from_str(label.as_str()));
        }
        let previous = match metric {
            MetricKind::Cpu => self.ivars().selected_cpu_label.replace(label.clone()),
            MetricKind::Ram => self.ivars().selected_ram_label.replace(label.clone()),
        };
        if label != previous {
            self.send(match metric {
                MetricKind::Cpu => IndicatorPreferenceChange::SetCpuLabel(label),
                MetricKind::Ram => IndicatorPreferenceChange::SetRamLabel(label),
            });
        }
    }

    fn apply_label_spacing(&self, value: isize) {
        if self.ivars().applying.get() {
            return;
        }
        let spacing = u8::try_from(value)
            .ok()
            .and_then(|value| LabelSpacing::try_from(value).ok());
        let Some(spacing) = spacing else {
            self.ivars().label_spacing.setIntegerValue(
                isize::try_from(self.ivars().selected_label_spacing.get().spaces())
                    .expect("label spacing fits NSInteger"),
            );
            return;
        };
        let value = match spacing.spaces() {
            1 => "1 espaço".to_owned(),
            spaces => format!("{spaces} espaços"),
        };
        set_slider_value_text(
            &self.ivars().label_spacing,
            &self.ivars().label_spacing_value,
            &value,
        );
        if spacing != self.ivars().selected_label_spacing.replace(spacing) {
            self.send(IndicatorPreferenceChange::SetLabelSpacing(spacing));
        }
    }
}

fn restore_identifier_segment(control: &NSSegmentedControl, mode: MetricIdentifierMode) {
    control.setSelectedSegment(match mode {
        MetricIdentifierMode::Text => 0,
        MetricIdentifierMode::SystemSymbol => 1,
        MetricIdentifierMode::Png => 2,
    });
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
    identifiers_heading: Retained<NSTextField>,
    cpu_identifier: MetricIdentifierControls,
    ram_identifier: MetricIdentifierControls,
    system_symbol_size_label: Retained<NSTextField>,
    system_symbol_size: Retained<NSSlider>,
    system_symbol_size_value: Retained<NSTextField>,
    reset_identifiers: Retained<NSButton>,
    labels_heading: Retained<NSTextField>,
    labels_visible: Retained<NSButton>,
    cpu_label_text: Retained<NSTextField>,
    cpu_label_field: Retained<NSTextField>,
    ram_label_text: Retained<NSTextField>,
    ram_label_field: Retained<NSTextField>,
    label_spacing_text: Retained<NSTextField>,
    label_spacing: Retained<NSSlider>,
    label_spacing_value: Retained<NSTextField>,
    labels_mode: Retained<NSSegmentedControl>,
    reset_labels: Retained<NSButton>,
    labels_editor: Retained<NSStackView>,
    typography_heading: Retained<NSTextField>,
    family_label: Retained<NSTextField>,
    font_family: Retained<NSButton>,
    size_label: Retained<NSTextField>,
    font_size: Retained<NSSlider>,
    font_size_value: Retained<NSTextField>,
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

struct MetricIdentifierControls {
    metric: MetricKind,
    label: Retained<NSTextField>,
    mode: Retained<NSSegmentedControl>,
    symbol: Retained<NSPopUpButton>,
    choose_png: Retained<NSButton>,
    thumbnail: Retained<NSImageView>,
    status: Retained<NSTextField>,
    remove: Retained<NSButton>,
}

#[derive(Clone, Debug)]
pub(super) struct ThumbnailAssetPlan {
    store: IconAssetStore,
}

impl ThumbnailAssetPlan {
    pub(super) fn new(store: IconAssetStore) -> Self {
        Self { store }
    }

    fn path_for(&self, metric: MetricKind, should_load: bool) -> Option<PathBuf> {
        should_load.then(|| self.store.path_for(metric))
    }
}

impl MetricIdentifierControls {
    fn retained(&self) -> Self {
        Self {
            metric: self.metric,
            label: self.label.clone(),
            mode: self.mode.clone(),
            symbol: self.symbol.clone(),
            choose_png: self.choose_png.clone(),
            thumbnail: self.thumbnail.clone(),
            status: self.status.clone(),
            remove: self.remove.clone(),
        }
    }

    fn set_frames(
        &self,
        row: statlet::preferences_view::RowSlot,
        detail: statlet::preferences_view::VerticalSlot,
        error: Option<statlet::preferences_view::VerticalSlot>,
        content_height: f64,
    ) {
        set_slot_frame(
            &self.label,
            row.label_x(),
            row.label_origin_y(content_height),
            row.height(),
        );
        set_slot_frame(
            &self.mode,
            row.control_x(),
            row.control_origin_y(content_height),
            row.height(),
        );
        let y = detail.origin_y(content_height);
        set_slot_frame(&self.symbol, 100.0, y, detail.height());
        set_slot_frame(&self.choose_png, 100.0, y, detail.height());
        set_slot_frame(&self.thumbnail, 228.0, y, detail.height());
        if let Some(error) = error {
            let message = statlet::preferences_view::MessageLayout::identifier_transaction_error();
            self.status.setFrame(NSRect::new(
                NSPoint::new(message.x(), error.origin_y(content_height)),
                NSSize::new(message.width(), error.height()),
            ));
            self.status
                .setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByWordWrapping);
            self.status.setMaximumNumberOfLines(message.maximum_lines());
        } else {
            self.status.setFrame(NSRect::new(
                NSPoint::new(266.0, y),
                NSSize::new(194.0, detail.height()),
            ));
            self.status
                .setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
            self.status.setMaximumNumberOfLines(1);
        }
        set_slot_frame(&self.remove, 468.0, y, detail.height());
    }

    fn views(&self) -> [&NSView; 7] {
        [
            &self.label,
            &self.mode,
            &self.symbol,
            &self.choose_png,
            &self.thumbnail,
            &self.status,
            &self.remove,
        ]
    }

    fn apply(
        &self,
        preferences: &statlet::indicator_preferences::MetricIdentifierPreferences,
        error: Option<&str>,
        processing: bool,
        thumbnail_assets: &ThumbnailAssetPlan,
    ) {
        restore_identifier_segment(&self.mode, preferences.mode);
        if let Some(index) = SystemSymbolName::curated_options()
            .iter()
            .position(|option| option.name() == preferences.system_symbol.as_str())
        {
            self.symbol
                .selectItemAtIndex(isize::try_from(index).expect("curated symbol index fits"));
        }
        let presentation =
            MetricIdentifierControlPresentation::with_processing(preferences, error, processing);
        self.symbol.setHidden(true);
        self.choose_png.setHidden(true);
        self.thumbnail.setHidden(true);
        self.status.setHidden(true);
        self.remove.setHidden(true);
        self.thumbnail.setImage(None);

        match presentation.detail {
            IdentifierDetailPresentation::Hidden => {}
            IdentifierDetailPresentation::SystemSymbol { .. } => {
                self.symbol.setHidden(false);
            }
            IdentifierDetailPresentation::Png {
                source_name,
                can_remove,
            } => {
                self.choose_png.setHidden(false);
                self.choose_png.setEnabled(!presentation.processing);
                self.status.setHidden(false);
                self.remove.setHidden(false);
                self.remove
                    .setEnabled(can_remove && !presentation.processing);
                let status = if presentation.processing {
                    Some("Processando PNG…")
                } else {
                    presentation.error.as_deref().or(source_name.as_deref())
                };
                self.status
                    .setStringValue(&objc2_foundation::NSString::from_str(
                        status.unwrap_or("Nenhum PNG escolhido."),
                    ));
                let status_color = if presentation.error.is_some() {
                    NSColor::systemOrangeColor()
                } else {
                    NSColor::secondaryLabelColor()
                };
                self.status.setTextColor(Some(&status_color));
                self.status
                    .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(
                        status.unwrap_or("Nenhum PNG escolhido."),
                    )));
                if let Some(path) = thumbnail_assets
                    .path_for(self.metric, can_remove && !presentation.processing)
                    .as_deref()
                    .and_then(Path::to_str)
                {
                    if let Some(image) = NSImage::initWithContentsOfFile(
                        NSImage::alloc(),
                        &objc2_foundation::NSString::from_str(path),
                    ) {
                        self.thumbnail.setImage(Some(&image));
                        self.thumbnail.setHidden(false);
                    }
                }
            }
        }
        if let Some(error) = presentation.error.as_deref() {
            self.status.setHidden(false);
            self.status
                .setStringValue(&objc2_foundation::NSString::from_str(error));
            self.status
                .setTextColor(Some(&NSColor::systemOrangeColor()));
            self.status
                .setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(error)));
        }
    }
}

pub(super) struct IndicatorAreaViews {
    colors: Retained<NSView>,
    labels: Retained<NSView>,
    typography: Retained<NSView>,
    refresh: Retained<NSView>,
}

impl IndicatorAreaViews {
    pub(super) fn set_visible_area(&self, area: PreferencesArea) {
        let visibility = IndicatorAreaVisibility::for_area(area);
        self.colors.setHidden(!visibility.colors);
        self.labels.setHidden(!visibility.labels);
        self.typography.setHidden(!visibility.typography);
        self.refresh.setHidden(!visibility.refresh);
    }

    fn set_frame_size(&self, size: NSSize) {
        self.colors.setFrameSize(size);
        self.labels.setFrameSize(size);
        self.typography.setFrameSize(size);
        self.refresh.setFrameSize(size);
    }

    fn retained(&self) -> Self {
        Self {
            colors: self.colors.clone(),
            labels: self.labels.clone(),
            typography: self.typography.clone(),
            refresh: self.refresh.clone(),
        }
    }
}

fn set_slot_frame(view: &NSView, x: f64, y: f64, height: f64) {
    view.setFrame(NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(view.frame().size.width, height),
    ));
}

fn shift_views_y(views: &[&NSView], delta: f64) {
    for view in views {
        let mut origin = view.frame().origin;
        origin.y += delta;
        view.setFrameOrigin(origin);
    }
}

impl IndicatorLayoutViews {
    fn apply(&self, layout: &IndicatorControlsLayout) -> f64 {
        let content_height = layout.content_height();

        let colors_heading = layout.colors_heading();
        set_slot_frame(
            &self.colors_heading,
            0.0,
            colors_heading.origin_y(content_height),
            colors_heading.height(),
        );
        let colors_reset = layout.colors_reset();
        self.reset_cpu_and_ram.setFrame(NSRect::new(
            NSPoint::new(colors_reset.x(), colors_reset.origin_y(content_height)),
            NSSize::new(colors_reset.width(), colors_reset.height()),
        ));

        let cpu_row = layout.cpu_row();
        set_slot_frame(
            &self.cpu_label,
            cpu_row.label_x(),
            cpu_row.label_origin_y(content_height),
            cpu_row.height(),
        );
        set_slot_frame(
            &self.cpu_mode,
            cpu_row.control_x(),
            cpu_row.control_origin_y(content_height),
            cpu_row.height(),
        );
        if let Some(cpu_editor) = layout.cpu_editor() {
            self.cpu_editor.setFrame(NSRect::new(
                NSPoint::new(0.0, cpu_editor.origin_y(content_height)),
                NSSize::new(540.0, cpu_editor.height()),
            ));
        }

        let ram_row = layout.ram_row();
        set_slot_frame(
            &self.ram_label,
            ram_row.label_x(),
            ram_row.label_origin_y(content_height),
            ram_row.height(),
        );
        set_slot_frame(
            &self.ram_mode,
            ram_row.control_x(),
            ram_row.control_origin_y(content_height),
            ram_row.height(),
        );
        if let Some(ram_editor) = layout.ram_editor() {
            self.ram_editor.setFrame(NSRect::new(
                NSPoint::new(0.0, ram_editor.origin_y(content_height)),
                NSSize::new(540.0, ram_editor.height()),
            ));
        }

        let identifiers_heading = layout.identifiers_heading();
        set_slot_frame(
            &self.identifiers_heading,
            0.0,
            identifiers_heading.origin_y(content_height),
            identifiers_heading.height(),
        );
        self.cpu_identifier.set_frames(
            layout.cpu_identifier_row(),
            layout.cpu_identifier_detail(),
            layout.cpu_identifier_error(),
            content_height,
        );
        self.ram_identifier.set_frames(
            layout.ram_identifier_row(),
            layout.ram_identifier_detail(),
            layout.ram_identifier_error(),
            content_height,
        );
        let symbol_size_row = layout.system_symbol_size_row();
        let symbol_size_y = symbol_size_row.origin_y(content_height);
        set_slot_frame(
            &self.system_symbol_size_label,
            symbol_size_row.label_x(),
            symbol_size_y,
            symbol_size_row.height(),
        );
        set_slot_frame(
            &self.system_symbol_size,
            132.0,
            symbol_size_y,
            symbol_size_row.height(),
        );
        set_slot_frame(
            &self.system_symbol_size_value,
            348.0,
            symbol_size_y,
            symbol_size_row.height(),
        );
        let identifiers_reset = layout.identifiers_reset();
        self.reset_identifiers.setFrame(NSRect::new(
            NSPoint::new(
                identifiers_reset.x(),
                identifiers_reset.origin_y(content_height),
            ),
            NSSize::new(identifiers_reset.width(), identifiers_reset.height()),
        ));

        let labels_heading = layout.labels_heading();
        set_slot_frame(
            &self.labels_heading,
            0.0,
            labels_heading.origin_y(content_height),
            labels_heading.height(),
        );
        let labels_reset = layout.labels_reset();
        self.reset_labels.setFrame(NSRect::new(
            NSPoint::new(labels_reset.x(), labels_reset.origin_y(content_height)),
            NSSize::new(labels_reset.width(), labels_reset.height()),
        ));
        let labels_visibility = layout.labels_visibility_row();
        let labels_visibility_y = labels_visibility.origin_y(content_height);
        set_slot_frame(
            &self.labels_visible,
            labels_visibility.label_x(),
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.cpu_label_text,
            90.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.cpu_label_field,
            128.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.ram_label_text,
            194.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.ram_label_field,
            234.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.label_spacing_text,
            300.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.label_spacing,
            346.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        set_slot_frame(
            &self.label_spacing_value,
            502.0,
            labels_visibility_y,
            labels_visibility.height(),
        );
        let labels_mode = layout.labels_mode_row();
        set_slot_frame(
            &self.labels_mode,
            labels_mode.control_x(),
            labels_mode.control_origin_y(content_height),
            labels_mode.height(),
        );
        if let Some(labels_editor) = layout.labels_editor() {
            self.labels_editor.setFrame(NSRect::new(
                NSPoint::new(0.0, labels_editor.origin_y(content_height)),
                NSSize::new(540.0, labels_editor.height()),
            ));
        }

        let typography_heading = layout.typography_heading();
        set_slot_frame(
            &self.typography_heading,
            0.0,
            typography_heading.origin_y(content_height),
            typography_heading.height(),
        );
        let family_row = layout.family_row();
        let family_y = family_row.origin_y(content_height);
        set_slot_frame(
            &self.family_label,
            family_row.label_x(),
            family_y,
            family_row.height(),
        );
        set_slot_frame(
            &self.font_family,
            family_row.control_x(),
            family_y,
            family_row.height(),
        );
        let size_row = layout.size_row();
        let size_y = size_row.origin_y(content_height);
        set_slot_frame(
            &self.size_label,
            size_row.label_x(),
            size_y,
            size_row.height(),
        );
        set_slot_frame(
            &self.font_size,
            size_row.control_x(),
            size_y,
            size_row.height(),
        );
        set_slot_frame(&self.font_size_value, 348.0, size_y, size_row.height());
        let weight_row = layout.weight_row();
        let weight_y = weight_row.origin_y(content_height);
        set_slot_frame(
            &self.weight_label,
            weight_row.label_x(),
            weight_y,
            weight_row.height(),
        );
        set_slot_frame(
            &self.font_weight,
            weight_row.control_x(),
            weight_y,
            weight_row.height(),
        );
        set_slot_frame(&self.reset_typography, 400.0, weight_y, weight_row.height());
        let font_fallback_warning = layout.font_fallback_warning();
        set_slot_frame(
            &self.font_fallback_warning,
            100.0,
            font_fallback_warning.origin_y(content_height),
            font_fallback_warning.height(),
        );
        let layout_warning = layout.layout_warning();
        set_slot_frame(
            &self.layout_warning,
            100.0,
            layout_warning.origin_y(content_height),
            layout_warning.height(),
        );

        let update_heading = layout.update_heading();
        set_slot_frame(
            &self.update_heading,
            0.0,
            update_heading.origin_y(content_height),
            update_heading.height(),
        );
        let interval_row = layout.interval_row();
        let interval_y = interval_row.origin_y(content_height);
        set_slot_frame(
            &self.interval_label,
            interval_row.label_x(),
            interval_y,
            interval_row.height(),
        );
        set_slot_frame(
            &self.interval_field,
            interval_row.control_x(),
            interval_y,
            interval_row.height(),
        );
        set_slot_frame(
            &self.interval_stepper,
            164.0,
            interval_y,
            interval_row.height(),
        );
        set_slot_frame(
            &self.seconds_label,
            194.0,
            interval_y,
            interval_row.height(),
        );
        set_slot_frame(
            &self.reset_refresh_interval,
            350.0,
            interval_y,
            interval_row.height(),
        );
        let interval_help = layout.interval_help();
        set_slot_frame(
            &self.interval_help,
            0.0,
            interval_help.origin_y(content_height),
            interval_help.height(),
        );
        let interval_error = layout.interval_error();
        set_slot_frame(
            &self.interval_error,
            100.0,
            interval_error.origin_y(content_height),
            interval_error.height(),
        );

        let colors_start = layout.colors_heading().top();
        let typography_start = layout.typography_heading().top();
        let refresh_start = layout.update_heading().top();
        let page_height = layout.page_height();

        shift_views_y(
            &[
                &self.colors_heading,
                &self.reset_cpu_and_ram,
                &self.cpu_label,
                &self.cpu_mode,
                &self.cpu_editor,
                &self.ram_label,
                &self.ram_mode,
                &self.ram_editor,
            ],
            page_height - content_height + colors_start,
        );
        shift_views_y(
            &[
                &self.identifiers_heading,
                self.cpu_identifier.views()[0],
                self.cpu_identifier.views()[1],
                self.cpu_identifier.views()[2],
                self.cpu_identifier.views()[3],
                self.cpu_identifier.views()[4],
                self.cpu_identifier.views()[5],
                self.cpu_identifier.views()[6],
                self.ram_identifier.views()[0],
                self.ram_identifier.views()[1],
                self.ram_identifier.views()[2],
                self.ram_identifier.views()[3],
                self.ram_identifier.views()[4],
                self.ram_identifier.views()[5],
                self.ram_identifier.views()[6],
                &self.system_symbol_size_label,
                &self.system_symbol_size,
                &self.system_symbol_size_value,
                &self.reset_identifiers,
                &self.labels_heading,
                &self.labels_visible,
                &self.cpu_label_text,
                &self.cpu_label_field,
                &self.ram_label_text,
                &self.ram_label_field,
                &self.label_spacing_text,
                &self.label_spacing,
                &self.label_spacing_value,
                &self.labels_mode,
                &self.reset_labels,
                &self.labels_editor,
            ],
            layout.labels_page_origin_y(layout.identifiers_heading())
                - layout.identifiers_heading().origin_y(content_height),
        );
        shift_views_y(
            &[
                &self.typography_heading,
                &self.family_label,
                &self.font_family,
                &self.size_label,
                &self.font_size,
                &self.font_size_value,
                &self.weight_label,
                &self.font_weight,
                &self.reset_typography,
                &self.font_fallback_warning,
                &self.layout_warning,
            ],
            page_height - content_height + typography_start,
        );
        shift_views_y(
            &[
                &self.update_heading,
                &self.interval_label,
                &self.interval_field,
                &self.interval_stepper,
                &self.seconds_label,
                &self.reset_refresh_interval,
                &self.interval_help,
                &self.interval_error,
            ],
            page_height - content_height + refresh_start,
        );

        page_height
    }
}

pub(super) struct IndicatorControls {
    view: Retained<NSStackView>,
    area_views: IndicatorAreaViews,
    layout_views: IndicatorLayoutViews,
    layout_visibility: Cell<Option<IndicatorControlsVisibility>>,
    content_height: Cell<f64>,
    cpu_mode: Retained<NSSegmentedControl>,
    reset_cpu_and_ram: Retained<NSButton>,
    ram_mode: Retained<NSSegmentedControl>,
    cpu_identifier: MetricIdentifierControls,
    ram_identifier: MetricIdentifierControls,
    thumbnail_assets: ThumbnailAssetPlan,
    system_symbol_size: Retained<NSSlider>,
    reset_identifiers: Retained<NSButton>,
    labels_visible: Retained<NSButton>,
    label_spacing: Retained<NSSlider>,
    labels_mode: Retained<NSSegmentedControl>,
    reset_labels: Retained<NSButton>,
    font_family: Retained<NSButton>,
    font_size: Retained<NSSlider>,
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
    pub(super) fn new(
        mtm: MainThreadMarker,
        proxy: EventLoopProxy<RuntimeEvent>,
        presentation: RuntimePresentation,
        thumbnail_assets: ThumbnailAssetPlan,
    ) -> Self {
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
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
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

        let identifiers_heading = heading(mtm, "Identificadores", 0.0);
        let cpu_identifier = metric_identifier_controls(mtm, MetricKind::Cpu);
        let ram_identifier = metric_identifier_controls(mtm, MetricKind::Ram);
        let system_symbol_size_label = text_label(mtm, "Tamanho do ícone", 0.0);
        system_symbol_size_label.setFrameSize(NSSize::new(124.0, 28.0));
        let system_symbol_size = NSSlider::initWithFrame(
            NSSlider::alloc(mtm),
            NSRect::new(NSPoint::new(132.0, 0.0), NSSize::new(208.0, 28.0)),
        );
        configure_discrete_slider(&system_symbol_size, system_symbol_size_slider_contract());
        let system_symbol_size_value = NSTextField::labelWithString(ns_string!("12 pt"), mtm);
        system_symbol_size_value.setFrame(NSRect::new(
            NSPoint::new(348.0, 0.0),
            NSSize::new(58.0, 28.0),
        ));
        system_symbol_size_value.setAccessibilityIdentifier(Some(ns_string!(
            "indicator.identifiers.system-symbol-size.value"
        )));
        let reset_identifiers = reset_button(
            mtm,
            "Restaurar identificadores",
            NSRect::new(NSPoint::new(350.0, 0.0), NSSize::new(200.0, 28.0)),
        );
        reset_identifiers
            .setAccessibilityIdentifier(Some(ns_string!("indicator.identifiers.reset")));
        let labels_heading = heading(mtm, "Rótulos", 0.0);
        let labels_visible = unsafe {
            NSButton::checkboxWithTitle_target_action(
                ns_string!("Mostrar rótulos C/R"),
                None,
                None,
                mtm,
            )
        };
        labels_visible.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(84.0, 24.0)));
        labels_visible.setTitle(ns_string!("Mostrar"));
        labels_visible.setAccessibilityLabel(Some(ns_string!("Mostrar rótulos C/R")));
        labels_visible.setAccessibilityIdentifier(Some(ns_string!("indicator.labels.visible")));
        let cpu_label_text = text_label(mtm, "CPU", 0.0);
        cpu_label_text.setFrameSize(NSSize::new(34.0, 28.0));
        let cpu_label_field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(128.0, 0.0), NSSize::new(60.0, 28.0)),
        );
        cpu_label_field.setAccessibilityLabel(Some(ns_string!("Rótulo de CPU, até 10 caracteres")));
        cpu_label_field.setAccessibilityIdentifier(Some(ns_string!("indicator.labels.cpu")));
        let ram_label_text = text_label(mtm, "RAM", 0.0);
        ram_label_text.setFrameSize(NSSize::new(38.0, 28.0));
        let ram_label_field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(234.0, 0.0), NSSize::new(60.0, 28.0)),
        );
        ram_label_field.setAccessibilityLabel(Some(ns_string!("Rótulo de RAM, até 10 caracteres")));
        ram_label_field.setAccessibilityIdentifier(Some(ns_string!("indicator.labels.ram")));
        let label_spacing_text = text_label(mtm, "Esp.", 0.0);
        label_spacing_text.setFrameSize(NSSize::new(42.0, 28.0));
        let label_spacing = NSSlider::initWithFrame(
            NSSlider::alloc(mtm),
            NSRect::new(NSPoint::new(346.0, 0.0), NSSize::new(148.0, 28.0)),
        );
        configure_discrete_slider(&label_spacing, label_spacing_slider_contract());
        let label_spacing_value = NSTextField::labelWithString(ns_string!("1 espaço"), mtm);
        label_spacing_value.setFrame(NSRect::new(
            NSPoint::new(502.0, 0.0),
            NSSize::new(58.0, 28.0),
        ));
        label_spacing_value
            .setAccessibilityIdentifier(Some(ns_string!("indicator.labels.spacing.value")));
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
        let font_size = NSSlider::initWithFrame(
            NSSlider::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(240.0, 28.0)),
        );
        configure_discrete_slider(&font_size, font_size_slider_contract());
        let font_size_value = NSTextField::labelWithString(ns_string!("12 pt"), mtm);
        font_size_value.setFrame(NSRect::new(
            NSPoint::new(348.0, 0.0),
            NSSize::new(58.0, 28.0),
        ));
        font_size_value.setAccessibilityIdentifier(Some(ns_string!("indicator.font.size.value")));

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
        font_fallback_warning.setAccessibilityIdentifier(Some(
            &objc2_foundation::NSString::from_str(
                TypographyWarningKind::FontFallback.accessibility_identifier(),
            ),
        ));
        let layout_warning = warning_label(mtm, 0.0);
        layout_warning.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            TypographyWarningKind::Layout.accessibility_identifier(),
        )));

        let update_heading = heading(mtm, "Atualização", 0.0);
        let interval_label = text_label(mtm, "Intervalo", 0.0);
        let interval_field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(58.0, 26.0)),
        );
        interval_field.setStringValue(ns_string!("2"));
        interval_field.setAccessibilityLabel(Some(ns_string!("Intervalo de atualização")));
        interval_field.setAccessibilityIdentifier(Some(ns_string!("indicator.refresh.interval")));
        let interval_format = IntervalFieldFormat::seconds();
        interval_field.setAccessibilityHelp(Some(&objc2_foundation::NSString::from_str(&format!(
            "Inteiro de {} a {} segundos",
            interval_format.minimum(),
            interval_format.maximum()
        ))));
        configure_interval_field(&interval_field, interval_format);
        let interval_stepper = NSStepper::initWithFrame(
            NSStepper::alloc(mtm),
            NSRect::new(NSPoint::new(164.0, 0.0), NSSize::new(20.0, 28.0)),
        );
        interval_stepper.setMinValue(interval_format.minimum().into());
        interval_stepper.setMaxValue(interval_format.maximum().into());
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
                presentation,
                applying: Cell::new(false),
                selected_family: RefCell::new(defaults.typography.family.clone()),
                selected_font_size: Cell::new(defaults.typography.size),
                interval_draft: RefCell::new(IntervalDraft::new(defaults.refresh_interval)),
                font_resources: RefCell::new(None),
                selected_cpu_label: RefCell::new(defaults.labels.cpu.clone()),
                selected_ram_label: RefCell::new(defaults.labels.ram.clone()),
                selected_label_spacing: Cell::new(defaults.labels.spacing),
                selected_cpu_identifier_mode: Cell::new(defaults.identifiers.cpu.mode),
                selected_ram_identifier_mode: Cell::new(defaults.identifiers.ram.mode),
                selected_system_symbol_size: Cell::new(defaults.identifiers.system_symbol_size),
                cpu_png_available: Cell::new(defaults.identifiers.cpu.png.is_some()),
                ram_png_available: Cell::new(defaults.identifiers.ram.png.is_some()),
                system_symbol_size: system_symbol_size.clone(),
                system_symbol_size_value: system_symbol_size_value.clone(),
                cpu_label_field: cpu_label_field.clone(),
                ram_label_field: ram_label_field.clone(),
                label_spacing: label_spacing.clone(),
                label_spacing_value: label_spacing_value.clone(),
                font_size: font_size.clone(),
                font_size_value: font_size_value.clone(),
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
            &cpu_identifier,
            &ram_identifier,
            &system_symbol_size,
            &reset_identifiers,
            &labels_visible,
            &cpu_label_field,
            &ram_label_field,
            &label_spacing,
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

        let colors_page = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 0.0)),
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
        ] {
            colors_page.addSubview(child);
        }
        let labels_page = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 0.0)),
        );
        labels_page.addSubview(&identifiers_heading);
        for child in cpu_identifier
            .views()
            .into_iter()
            .chain(ram_identifier.views())
        {
            labels_page.addSubview(child);
        }
        labels_page.addSubview(&system_symbol_size_label);
        labels_page.addSubview(&system_symbol_size);
        labels_page.addSubview(&system_symbol_size_value);
        labels_page.addSubview(&reset_identifiers);
        for child in [
            &*labels_heading as &NSView,
            &*labels_visible,
            &*cpu_label_text,
            &*cpu_label_field,
            &*ram_label_text,
            &*ram_label_field,
            &*label_spacing_text,
            &*label_spacing,
            &*label_spacing_value,
            &*labels_mode,
            &*reset_labels,
            labels_editor.view(),
        ] {
            labels_page.addSubview(child);
        }
        let typography_page = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 0.0)),
        );
        for child in [
            &*typography_heading as &NSView,
            &*family_label,
            &*font_family,
            &*size_label,
            &*font_size,
            &*font_size_value,
            &*weight_label,
            &*font_weight,
            &*reset_typography,
            &*font_fallback_warning,
            &*layout_warning,
        ] {
            typography_page.addSubview(child);
        }
        let refresh_page = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 0.0)),
        );
        for child in [
            &*update_heading as &NSView,
            &*interval_label,
            &*interval_field,
            &*interval_stepper,
            &*reset_refresh_interval,
            &*seconds_label,
            &*interval_help,
            &*interval_error,
        ] {
            refresh_page.addSubview(child);
        }
        let area_views = IndicatorAreaViews {
            colors: colors_page,
            labels: labels_page,
            typography: typography_page,
            refresh: refresh_page,
        };
        for page in [
            &*area_views.colors,
            &*area_views.labels,
            &*area_views.typography,
            &*area_views.refresh,
        ] {
            view.addSubview(page);
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
            identifiers_heading: identifiers_heading.clone(),
            cpu_identifier: cpu_identifier.retained(),
            ram_identifier: ram_identifier.retained(),
            system_symbol_size_label: system_symbol_size_label.clone(),
            system_symbol_size: system_symbol_size.clone(),
            system_symbol_size_value: system_symbol_size_value.clone(),
            reset_identifiers: reset_identifiers.clone(),
            labels_heading: labels_heading.clone(),
            labels_visible: labels_visible.clone(),
            cpu_label_text: cpu_label_text.clone(),
            cpu_label_field: cpu_label_field.clone(),
            ram_label_text: ram_label_text.clone(),
            ram_label_field: ram_label_field.clone(),
            label_spacing_text: label_spacing_text.clone(),
            label_spacing: label_spacing.clone(),
            label_spacing_value: label_spacing_value.clone(),
            labels_mode: labels_mode.clone(),
            reset_labels: reset_labels.clone(),
            labels_editor: labels_editor.view().retain(),
            typography_heading: typography_heading.clone(),
            family_label: family_label.clone(),
            font_family: font_family.clone(),
            size_label: size_label.clone(),
            font_size: font_size.clone(),
            font_size_value: font_size_value.clone(),
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
            area_views,
            layout_views,
            layout_visibility: Cell::new(None),
            content_height: Cell::new(0.0),
            cpu_mode,
            reset_cpu_and_ram,
            ram_mode,
            cpu_identifier,
            ram_identifier,
            thumbnail_assets,
            system_symbol_size,
            reset_identifiers,
            labels_visible,
            label_spacing,
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
        controls.apply(&IndicatorPreferences::default(), None, None, false, false);
        controls
            .area_views()
            .set_visible_area(PreferencesArea::Colors);
        controls
    }

    pub(super) fn apply(
        &self,
        preferences: &IndicatorPreferences,
        cpu_icon_error: Option<&str>,
        ram_icon_error: Option<&str>,
        cpu_icon_pending: bool,
        ram_icon_pending: bool,
    ) {
        self.target.ivars().applying.set(true);
        self.target
            .ivars()
            .selected_cpu_identifier_mode
            .set(preferences.identifiers.cpu.mode);
        self.target
            .ivars()
            .selected_ram_identifier_mode
            .set(preferences.identifiers.ram.mode);
        self.target
            .ivars()
            .cpu_png_available
            .set(preferences.identifiers.cpu.png.is_some());
        self.target
            .ivars()
            .ram_png_available
            .set(preferences.identifiers.ram.png.is_some());
        self.cpu_identifier.apply(
            &preferences.identifiers.cpu,
            cpu_icon_error,
            cpu_icon_pending,
            &self.thumbnail_assets,
        );
        self.ram_identifier.apply(
            &preferences.identifiers.ram,
            ram_icon_error,
            ram_icon_pending,
            &self.thumbnail_assets,
        );
        let identifier_editing = IdentifierEditingPresentation::new(
            preferences.identifiers.cpu.mode,
            preferences.identifiers.ram.mode,
        );
        self.target
            .ivars()
            .selected_system_symbol_size
            .set(preferences.identifiers.system_symbol_size);
        self.system_symbol_size
            .setIntegerValue(preferences.identifiers.system_symbol_size.points().into());
        self.system_symbol_size
            .setEnabled(identifier_editing.system_symbol_size_enabled());
        self.system_symbol_size
            .setAccessibilityHelp(Some(&objc2_foundation::NSString::from_str(
                identifier_editing.system_symbol_size_help(),
            )));
        set_slider_value_text(
            &self.system_symbol_size,
            &self.layout_views.system_symbol_size_value,
            &format!("{} pt", preferences.identifiers.system_symbol_size.points()),
        );
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
            .selected_cpu_label
            .replace(preferences.labels.cpu.clone());
        self.target
            .ivars()
            .selected_ram_label
            .replace(preferences.labels.ram.clone());
        self.target
            .ivars()
            .selected_label_spacing
            .set(preferences.labels.spacing);
        self.layout_views
            .cpu_label_field
            .setStringValue(&objc2_foundation::NSString::from_str(
                preferences.labels.cpu.as_str(),
            ));
        self.layout_views
            .ram_label_field
            .setStringValue(&objc2_foundation::NSString::from_str(
                preferences.labels.ram.as_str(),
            ));
        let label_editing = LabelEditingPresentation::new(
            preferences.identifiers.cpu.mode,
            preferences.identifiers.ram.mode,
        );
        self.layout_views
            .cpu_label_field
            .setEnabled(label_editing.cpu_enabled());
        self.layout_views.cpu_label_field.setAccessibilityHelp(
            label_editing
                .cpu_help()
                .map(objc2_foundation::NSString::from_str)
                .as_deref(),
        );
        self.layout_views
            .ram_label_field
            .setEnabled(label_editing.ram_enabled());
        self.layout_views.ram_label_field.setAccessibilityHelp(
            label_editing
                .ram_help()
                .map(objc2_foundation::NSString::from_str)
                .as_deref(),
        );
        self.label_spacing
            .setEnabled(label_editing.spacing_enabled());
        self.label_spacing.setAccessibilityHelp(
            label_editing
                .spacing_help()
                .map(objc2_foundation::NSString::from_str)
                .as_deref(),
        );
        self.label_spacing.setIntegerValue(
            isize::try_from(preferences.labels.spacing.spaces())
                .expect("label spacing fits NSInteger"),
        );
        let spacing_value = match preferences.labels.spacing.spaces() {
            1 => "1 espaço".to_owned(),
            spaces => format!("{spaces} espaços"),
        };
        set_slider_value_text(
            &self.label_spacing,
            &self.layout_views.label_spacing_value,
            &spacing_value,
        );
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
        let interval_error = interval.error().map(|error| error.message());
        set_inline_error(&self.layout_views.interval_error, interval_error.as_deref());
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
        let visibility = IndicatorControlsVisibility {
            cpu_editor: cpu_fixed,
            ram_editor: ram_fixed,
            labels_editor: labels_fixed,
            cpu_identifier_error: cpu_icon_error.is_some(),
            ram_identifier_error: ram_icon_error.is_some(),
        };
        if self.layout_visibility.get() != Some(visibility) {
            self.apply_layout(visibility);
            self.layout_visibility.set(Some(visibility));
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
                    &self.reset_cpu_and_ram,
                    &ram_state,
                );
            } else {
                self.ram_mode.setNextKeyView(Some(&self.reset_cpu_and_ram));
            }
            let identifier_control = |target| -> &NSView {
                match target {
                    IdentifierEditingFocusTarget::CpuMode => &self.cpu_identifier.mode,
                    IdentifierEditingFocusTarget::CpuSymbol => &self.cpu_identifier.symbol,
                    IdentifierEditingFocusTarget::CpuChoosePng => &self.cpu_identifier.choose_png,
                    IdentifierEditingFocusTarget::CpuRemovePng => &self.cpu_identifier.remove,
                    IdentifierEditingFocusTarget::RamMode => &self.ram_identifier.mode,
                    IdentifierEditingFocusTarget::RamSymbol => &self.ram_identifier.symbol,
                    IdentifierEditingFocusTarget::RamChoosePng => &self.ram_identifier.choose_png,
                    IdentifierEditingFocusTarget::RamRemovePng => &self.ram_identifier.remove,
                    IdentifierEditingFocusTarget::SystemSymbolSize => &self.system_symbol_size,
                    IdentifierEditingFocusTarget::ResetIdentifiers => &self.reset_identifiers,
                }
            };
            let identifier_focus_order = identifier_editing.focus_order(
                preferences.identifiers.cpu.png.is_some(),
                preferences.identifiers.ram.png.is_some(),
            );
            for link in identifier_focus_order.windows(2) {
                identifier_control(link[0]).setNextKeyView(Some(identifier_control(link[1])));
            }
            self.reset_identifiers
                .setNextKeyView(Some(&self.labels_visible));
            let label_control = |target| -> &NSView {
                match target {
                    LabelEditingFocusTarget::CpuLabel => &self.layout_views.cpu_label_field,
                    LabelEditingFocusTarget::RamLabel => &self.layout_views.ram_label_field,
                    LabelEditingFocusTarget::Spacing => &self.label_spacing,
                    LabelEditingFocusTarget::LabelColorMode => &self.labels_mode,
                }
            };
            let label_focus_order = label_editing.focus_order();
            self.labels_visible
                .setNextKeyView(Some(label_control(label_focus_order[0])));
            for link in label_focus_order.windows(2) {
                label_control(link[0]).setNextKeyView(Some(label_control(link[1])));
            }
            if labels_fixed {
                self.labels_editor.configure_key_order(
                    &self.labels_mode,
                    &self.reset_labels,
                    &labels_state,
                );
            } else {
                self.labels_mode.setNextKeyView(Some(&self.reset_labels));
            }
            self.font_family.setNextKeyView(Some(&self.font_size));
            self.font_size.setNextKeyView(Some(&self.font_weight));
            self.font_weight
                .setNextKeyView(Some(&self.reset_typography));
            self.interval_field
                .setNextKeyView(Some(&self.interval_stepper));
            self.interval_stepper
                .setNextKeyView(Some(&self.reset_refresh_interval));
        }
        self.target.ivars().applying.set(false);
    }

    fn apply_layout(&self, visibility: IndicatorControlsVisibility) {
        let layout = IndicatorControlsLayout::new(visibility);
        let page_height = self.layout_views.apply(&layout);
        let size = NSSize::new(560.0, page_height);
        self.view.setFrameSize(size);
        self.area_views.set_frame_size(size);
        self.content_height.set(page_height);
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

    pub(super) fn area_views(&self) -> IndicatorAreaViews {
        self.area_views.retained()
    }

    pub(super) fn first_key_view_for(&self, area: PreferencesArea) -> &NSView {
        match area {
            PreferencesArea::Colors => &self.cpu_mode,
            PreferencesArea::Labels => &self.cpu_identifier.mode,
            PreferencesArea::Typography => &self.font_family,
            PreferencesArea::Refresh => &self.interval_field,
            PreferencesArea::DiskAndMole => {
                unreachable!("disk preferences use their own first key view")
            }
        }
    }

    pub(super) fn last_key_views(&self) -> [&NSView; 4] {
        [
            &self.reset_cpu_and_ram,
            &self.reset_labels,
            &self.reset_typography,
            &self.reset_refresh_interval,
        ]
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
        set_typography_warning(
            &self.layout_views.font_fallback_warning,
            TypographyWarningKind::FontFallback,
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
        set_typography_warning(
            &self.layout_views.layout_warning,
            TypographyWarningKind::Layout,
            message,
        );
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
            .setIntegerValue(typography.size.points().into());
        set_slider_value_text(
            &self.font_size,
            &self.layout_views.font_size_value,
            &format!("{} pt", typography.size.points()),
        );
        self.font_weight
            .setSelectedSegment(match typography.weight {
                FontWeight::Regular => 0,
                FontWeight::Medium => 1,
                FontWeight::Bold => 2,
            });
    }
}

fn metric_identifier_controls(
    mtm: MainThreadMarker,
    metric: MetricKind,
) -> MetricIdentifierControls {
    let (metric_name, prefix) = match metric {
        MetricKind::Cpu => ("CPU", "indicator.cpu.identifier"),
        MetricKind::Ram => ("RAM", "indicator.ram.identifier"),
    };
    let label = text_label(mtm, metric_name, 0.0);
    let mode = segmented(
        mtm,
        &["Texto", "Ícone do macOS", "PNG"],
        &format!("{prefix}.mode"),
        NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(400.0, 28.0)),
        None,
        None,
    );
    mode.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Tipo de identificador de {metric_name}"
    ))));

    let symbol = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(100.0, 0.0), NSSize::new(300.0, 30.0)),
        false,
    );
    let labels = SystemSymbolName::curated_options()
        .iter()
        .map(|option| objc2_foundation::NSString::from_str(option.label_pt_br()))
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(|label| &**label).collect::<Vec<_>>();
    symbol.addItemsWithTitles(&NSArray::from_slice(&label_refs));
    for (index, option) in SystemSymbolName::curated_options().iter().enumerate() {
        let Some(item) = symbol.itemAtIndex(index as isize) else {
            continue;
        };
        let name = objc2_foundation::NSString::from_str(option.name());
        let accessibility = objc2_foundation::NSString::from_str(option.label_pt_br());
        if let Some(image) =
            NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, Some(&accessibility))
        {
            item.setImage(Some(&image));
        }
    }
    symbol.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{prefix}.symbol"
    ))));
    symbol.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Ícone do macOS para {metric_name}"
    ))));

    let choose_png = unsafe {
        NSButton::buttonWithTitle_target_action(ns_string!("Escolher PNG…"), None, None, mtm)
    };
    choose_png.setFrame(NSRect::new(
        NSPoint::new(100.0, 0.0),
        NSSize::new(120.0, 30.0),
    ));
    choose_png.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{prefix}.choose-png"
    ))));
    choose_png.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Escolher PNG para {metric_name}"
    ))));

    let thumbnail = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        NSRect::new(NSPoint::new(228.0, 0.0), NSSize::new(30.0, 30.0)),
    );
    thumbnail.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{prefix}.thumbnail"
    ))));
    thumbnail.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Miniatura do PNG de {metric_name}"
    ))));

    let status = NSTextField::labelWithString(ns_string!("Nenhum PNG escolhido."), mtm);
    status.setFrame(NSRect::new(
        NSPoint::new(266.0, 0.0),
        NSSize::new(194.0, 30.0),
    ));
    status.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
    status.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{prefix}.status"
    ))));

    let remove =
        unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Remover"), None, None, mtm) };
    remove.setFrame(NSRect::new(
        NSPoint::new(468.0, 0.0),
        NSSize::new(82.0, 30.0),
    ));
    remove.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{prefix}.remove"
    ))));
    remove.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Remover PNG de {metric_name}"
    ))));

    MetricIdentifierControls {
        metric,
        label,
        mode,
        symbol,
        choose_png,
        thumbnail,
        status,
        remove,
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
    field.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    field.sizeToFit();
    field.setFrame(NSRect::new(
        NSPoint::new(0.0, y),
        NSSize::new(fitted_heading_width(field.frame().size.width), 24.0),
    ));
    field
}

fn fitted_heading_width(intrinsic_width: f64) -> f64 {
    intrinsic_width.clamp(92.0, 378.0)
}

fn text_label(mtm: MainThreadMarker, title: &str, y: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(title), mtm);
    field.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(92.0, 24.0)));
    field
}

fn configure_interval_field(field: &NSTextField, format: IntervalFieldFormat) {
    let formatter = NSNumberFormatter::init(NSNumberFormatter::alloc());
    formatter.setNumberStyle(NSNumberFormatterStyle::DecimalStyle);
    formatter.setAllowsFloats(format.allows_floats());
    formatter.setUsesGroupingSeparator(format.uses_grouping_separator());
    formatter.setMinimum(Some(&NSNumber::numberWithInteger(format.minimum().into())));
    formatter.setMaximum(Some(&NSNumber::numberWithInteger(format.maximum().into())));
    formatter.setPartialStringValidationEnabled(format.validates_partial_input());
    field.setFormatter(Some(&formatter));
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
    cpu_identifier: &MetricIdentifierControls,
    ram_identifier: &MetricIdentifierControls,
    system_symbol_size: &NSSlider,
    reset_identifiers: &NSButton,
    labels_visible: &NSButton,
    cpu_label_field: &NSTextField,
    ram_label_field: &NSTextField,
    label_spacing: &NSSlider,
    labels_mode: &NSSegmentedControl,
    reset_labels: &NSButton,
    font_family: &NSButton,
    font_size: &NSSlider,
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
                &*cpu_identifier.mode as &objc2_app_kit::NSControl,
                sel!(changeCpuIdentifierMode:),
            ),
            (
                &*ram_identifier.mode as &objc2_app_kit::NSControl,
                sel!(changeRamIdentifierMode:),
            ),
            (
                &*cpu_identifier.symbol as &objc2_app_kit::NSControl,
                sel!(changeCpuSystemSymbol:),
            ),
            (
                &*ram_identifier.symbol as &objc2_app_kit::NSControl,
                sel!(changeRamSystemSymbol:),
            ),
            (
                &**system_symbol_size as &objc2_app_kit::NSControl,
                sel!(changeSystemSymbolSize:),
            ),
            (
                &*cpu_identifier.choose_png as &objc2_app_kit::NSControl,
                sel!(chooseCpuPng:),
            ),
            (
                &*ram_identifier.choose_png as &objc2_app_kit::NSControl,
                sel!(chooseRamPng:),
            ),
            (
                &*cpu_identifier.remove as &objc2_app_kit::NSControl,
                sel!(removeCpuPng:),
            ),
            (
                &*ram_identifier.remove as &objc2_app_kit::NSControl,
                sel!(removeRamPng:),
            ),
            (
                &**reset_identifiers as &objc2_app_kit::NSControl,
                sel!(resetIdentifiers:),
            ),
            (
                &**labels_visible as &objc2_app_kit::NSControl,
                sel!(toggleLabelsVisible:),
            ),
            (
                &**cpu_label_field as &objc2_app_kit::NSControl,
                sel!(commitCpuLabel:),
            ),
            (
                &**ram_label_field as &objc2_app_kit::NSControl,
                sel!(commitRamLabel:),
            ),
            (
                &**label_spacing as &objc2_app_kit::NSControl,
                sel!(changeLabelSpacing:),
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
                sel!(changeFontSize:),
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
        interval_field.setDelegate(Some(ProtocolObject::from_ref(target)));
        cpu_label_field.setDelegate(Some(ProtocolObject::from_ref(target)));
        ram_label_field.setDelegate(Some(ProtocolObject::from_ref(target)));
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

fn set_slider_value_text(slider: &NSSlider, value_label: &NSTextField, value: &str) {
    let value = objc2_foundation::NSString::from_str(value);
    value_label.setStringValue(&value);
    slider.setAccessibilityValueDescription(Some(&value));
}

fn set_inline_error(field: &NSTextField, message: Option<&str>) {
    set_warning_text(field, message);
}

fn set_typography_warning(field: &NSTextField, kind: TypographyWarningKind, message: Option<&str>) {
    set_warning_text(field, kind.accessibility_label(message));
}

fn set_warning_text(field: &NSTextField, message: Option<&str>) {
    field.setStringValue(&objc2_foundation::NSString::from_str(message.unwrap_or("")));
    field.setAccessibilityLabel(message.map(objc2_foundation::NSString::from_str).as_deref());
    field.setHidden(message.is_none());
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use statlet::icon_assets::IconAssetStore;
    use statlet::indicator_preferences::MetricKind;
    use statlet::preferences_view::PreferencesArea;
    use statlet::runtime_profile::{BundleProfileMetadata, RuntimeProfile};

    use super::{
        fitted_heading_width, get_or_create_font_resources, png_panel_presentation,
        IndicatorAreaVisibility,
    };

    #[test]
    fn each_indicator_sidebar_destination_exposes_only_its_retained_controls() {
        let cases = [
            (
                PreferencesArea::Colors,
                IndicatorAreaVisibility::new(true, false, false, false),
            ),
            (
                PreferencesArea::Labels,
                IndicatorAreaVisibility::new(false, true, false, false),
            ),
            (
                PreferencesArea::Typography,
                IndicatorAreaVisibility::new(false, false, true, false),
            ),
            (
                PreferencesArea::Refresh,
                IndicatorAreaVisibility::new(false, false, false, true),
            ),
        ];

        for (area, expected) in cases {
            assert_eq!(IndicatorAreaVisibility::for_area(area), expected);
            assert_eq!(
                [
                    expected.colors,
                    expected.labels,
                    expected.typography,
                    expected.refresh,
                ]
                .into_iter()
                .filter(|visible| *visible)
                .count(),
                1
            );
        }
    }

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

    #[test]
    fn injected_store_drives_the_preferences_thumbnail_plan() {
        let store = IconAssetStore::new(PathBuf::from(
            "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/indicator-icons",
        ));

        let plan = super::ThumbnailAssetPlan::new(store);

        assert_eq!(
            plan.path_for(MetricKind::Cpu, true),
            Some(PathBuf::from(
                "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/indicator-icons/cpu.png"
            ))
        );
        assert_eq!(plan.path_for(MetricKind::Cpu, false), None);
    }

    #[test]
    fn production_png_panel_copy_remains_byte_for_byte_unchanged() {
        let panel = png_panel_presentation(MetricKind::Cpu, &Default::default());

        assert_eq!(panel.title, None);
        assert_eq!(panel.prompt, "Escolher PNG");
        assert_eq!(
            panel.message,
            "Escolha um PNG com transparência. O Statlet reduzirá e otimizará uma cópia."
        );
    }

    #[test]
    fn development_png_panel_identifies_the_profile_in_title_and_message() {
        let profile = RuntimeProfile::resolve(BundleProfileMetadata {
            bundle_identifier: Some(
                "io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into(),
            ),
            runtime_profile: Some("development".into()),
            dev_instance_id: Some("task-a-0123456789ab".into()),
            dev_display_name: Some("Task A".into()),
            dev_short_marker: Some("0123".into()),
        })
        .unwrap();

        let panel = png_panel_presentation(MetricKind::Ram, &profile.presentation());

        assert_eq!(
            panel.title.as_deref(),
            Some("Escolher PNG da RAM — Dev 0123: Task A")
        );
        assert_eq!(panel.prompt, "Escolher PNG");
        assert_eq!(
            panel.message,
            "Statlet Dev — Task A (task-a-0123456789ab): Escolha um PNG com transparência. O Statlet reduzirá e otimizará uma cópia."
        );
    }

    #[test]
    fn heading_width_fits_its_intrinsic_content_without_reaching_neighbor_actions() {
        assert_eq!(fitted_heading_width(60.0), 92.0);
        assert_eq!(fitted_heading_width(120.0), 120.0);
        assert_eq!(fitted_heading_width(420.0), 378.0);
    }
}
