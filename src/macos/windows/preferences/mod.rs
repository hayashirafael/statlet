use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly, Message};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColorWell, NSControlStateValueOn, NSControlTextEditingDelegate,
    NSScrollView, NSSlider, NSStackView, NSTableColumn, NSTableView, NSTableViewDataSource,
    NSTableViewDelegate, NSTableViewStyle, NSTextField, NSUserInterfaceItemIdentifier, NSView,
    NSWindow, NSWindowDelegate,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize,
};
use statlet::core::{AppEvent, AppState, WarningThreshold};
use statlet::icon_assets::IconAssetStore;
use statlet::indicator_preferences::IndicatorPreferences;
use statlet::preferences_view::{
    preserve_scroll_origin_from_top, MessageLayout, PreferencesArea, PreferencesControlsCache,
    PreferencesNavigationPolicy, PreferencesShellFocusTarget, PreferencesShellPresentation,
};
use statlet::runtime_profile::RuntimePresentation;

use super::common::{threshold_title, ControlTarget, PreferencesWindowHost};
use super::{IndicatorFontFallback, IndicatorLayoutDiagnostics, IndicatorSurfaceUpdate};
use crate::macos::renderer::PreviewImages;

mod color_editor;
mod font_picker;
mod indicator;
pub(super) mod preview;

use indicator::{IndicatorAreaViews, IndicatorControls, ThumbnailAssetPlan};
use preview::PreviewPane;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PreferencesAreaState {
    visible: PreferencesArea,
}

impl PreferencesAreaState {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn visible(self) -> PreferencesArea {
        self.visible
    }

    pub(super) fn select(self, visible: PreferencesArea) -> Self {
        Self { visible }
    }
}

fn select_visible_area(
    state: &RefCell<PreferencesAreaState>,
    select_area: impl FnOnce(PreferencesArea),
) {
    let area = state.borrow().visible();
    select_area(area);
}

fn deactivate_inactive_color_wells<T>(
    color_wells: &[T],
    mut is_inactive: impl FnMut(&T) -> bool,
    mut deactivate: impl FnMut(&T),
) {
    for color_well in color_wells {
        if is_inactive(color_well) {
            deactivate(color_well);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreferencesRegion {
    IndicatorGroups,
    Previews,
    Footer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RegionPlacement {
    Scrollable,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PreferencesShellContract;

impl PreferencesShellContract {
    pub(super) const fn new() -> Self {
        Self
    }

    pub(super) const fn content_size(self) -> (f64, f64) {
        (860.0, 700.0)
    }

    pub(super) const fn sidebar_width(self) -> f64 {
        180.0
    }

    pub(super) const fn sidebar_header_visible(self) -> bool {
        false
    }

    pub(super) const fn sidebar_accessibility_identifier(self) -> &'static str {
        "preferences.sidebar"
    }

    pub(super) const fn placement(self, region: PreferencesRegion) -> RegionPlacement {
        match region {
            PreferencesRegion::IndicatorGroups => RegionPlacement::Scrollable,
            PreferencesRegion::Previews | PreferencesRegion::Footer => RegionPlacement::Fixed,
        }
    }

    pub(super) const fn accessibility_identifiers(self) -> [&'static str; 6] {
        [
            "preferences.area",
            "indicator.preview.light",
            "indicator.preview.dark",
            "indicator.reset.all",
            "indicator.reset.undo",
            "indicator.save.retry",
        ]
    }

    #[cfg(test)]
    pub(super) const fn identifier_accessibility_identifiers(self) -> [&'static str; 12] {
        [
            "indicator.cpu.identifier.mode",
            "indicator.cpu.identifier.symbol",
            "indicator.cpu.identifier.choose-png",
            "indicator.cpu.identifier.thumbnail",
            "indicator.cpu.identifier.status",
            "indicator.cpu.identifier.remove",
            "indicator.ram.identifier.mode",
            "indicator.ram.identifier.symbol",
            "indicator.ram.identifier.choose-png",
            "indicator.ram.identifier.thumbnail",
            "indicator.ram.identifier.status",
            "indicator.ram.identifier.remove",
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiscreteSliderContract {
    min: u8,
    max: u8,
    step: u8,
    ticks: NSInteger,
    identifier: &'static str,
    accessibility_label: &'static str,
    accessibility_help: &'static str,
}

impl DiscreteSliderContract {
    const fn new(
        min: u8,
        max: u8,
        step: u8,
        ticks: NSInteger,
        identifier: &'static str,
        accessibility_label: &'static str,
        accessibility_help: &'static str,
    ) -> Self {
        Self {
            min,
            max,
            step,
            ticks,
            identifier,
            accessibility_label,
            accessibility_help,
        }
    }

    pub(super) const fn min(self) -> u8 {
        self.min
    }

    pub(super) const fn max(self) -> u8 {
        self.max
    }

    pub(super) const fn step(self) -> u8 {
        self.step
    }

    pub(super) const fn ticks(self) -> NSInteger {
        self.ticks
    }

    pub(super) const fn identifier(self) -> &'static str {
        self.identifier
    }

    pub(super) const fn accessibility_label(self) -> &'static str {
        self.accessibility_label
    }

    pub(super) const fn accessibility_help(self) -> &'static str {
        self.accessibility_help
    }

    pub(super) const fn continuous(self) -> bool {
        true
    }

    pub(super) const fn tick_values_only(self) -> bool {
        true
    }
}

pub(super) const fn label_spacing_slider_contract() -> DiscreteSliderContract {
    DiscreteSliderContract::new(
        0,
        4,
        1,
        5,
        "indicator.labels.spacing",
        "Espaçamento entre rótulo e percentual",
        "Escolha de zero a quatro espaços.",
    )
}

pub(super) const fn font_size_slider_contract() -> DiscreteSliderContract {
    DiscreteSliderContract::new(
        9,
        14,
        1,
        6,
        "indicator.font.size",
        "Tamanho da fonte",
        "Escolha de nove a quatorze pontos.",
    )
}

pub(super) const fn system_symbol_size_slider_contract() -> DiscreteSliderContract {
    DiscreteSliderContract::new(
        8,
        14,
        1,
        7,
        "indicator.identifiers.system-symbol-size",
        "Tamanho do ícone",
        "Ajusta o tamanho compartilhado dos ícones do macOS de CPU e RAM.",
    )
}

pub(super) const fn disk_threshold_slider_contract() -> DiscreteSliderContract {
    DiscreteSliderContract::new(
        70,
        95,
        5,
        6,
        "disk.warning.threshold",
        "Limite de aviso do disco",
        "Escolha o percentual de ocupação que inicia a observação de pouco espaço.",
    )
}

pub(super) fn configure_discrete_slider(slider: &NSSlider, contract: DiscreteSliderContract) {
    slider.setMinValue(f64::from(contract.min()));
    slider.setMaxValue(f64::from(contract.max()));
    slider.setAltIncrementValue(f64::from(contract.step()));
    slider.setNumberOfTickMarks(contract.ticks());
    slider.setAllowsTickMarkValuesOnly(contract.tick_values_only());
    slider.setContinuous(contract.continuous());
    slider.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
        contract.identifier(),
    )));
    slider.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(
        contract.accessibility_label(),
    )));
    slider.setAccessibilityHelp(Some(&objc2_foundation::NSString::from_str(
        contract.accessibility_help(),
    )));
}

pub(super) fn warning_threshold_from_slider_value(value: NSInteger) -> Option<WarningThreshold> {
    u8::try_from(value)
        .ok()
        .and_then(|value| WarningThreshold::try_from(value).ok())
}

fn set_disk_threshold_value(
    slider: &NSSlider,
    value_label: &NSTextField,
    threshold: WarningThreshold,
) {
    let value = threshold_title(threshold);
    value_label.setStringValue(&value);
    slider.setAccessibilityValueDescription(Some(&value));
}

struct DiskThresholdTargetIvars {
    proxy: tao::event_loop::EventLoopProxy<crate::macos::RuntimeEvent>,
    applying: Cell<bool>,
    selected: Cell<WarningThreshold>,
    slider: Retained<NSSlider>,
    value_label: Retained<NSTextField>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DiskThresholdTargetIvars]
    struct DiskThresholdTarget;

    unsafe impl NSObjectProtocol for DiskThresholdTarget {}

    impl DiskThresholdTarget {
        #[unsafe(method(changeWarningThreshold:))]
        fn change_warning_threshold(&self, sender: &NSSlider) {
            if self.ivars().applying.get() {
                return;
            }
            let Some(threshold) = warning_threshold_from_slider_value(sender.integerValue()) else {
                sender.setIntegerValue(self.ivars().selected.get().get().into());
                return;
            };
            set_disk_threshold_value(sender, &self.ivars().value_label, threshold);
            if threshold == self.ivars().selected.replace(threshold) {
                return;
            }
            let _ = self
                .ivars()
                .proxy
                .send_event(crate::macos::RuntimeEvent::App(
                    AppEvent::SetWarningThreshold(threshold),
                ));
        }
    }
);

impl DiskThresholdTarget {
    fn new(
        mtm: MainThreadMarker,
        proxy: tao::event_loop::EventLoopProxy<crate::macos::RuntimeEvent>,
        slider: Retained<NSSlider>,
        value_label: Retained<NSTextField>,
    ) -> Retained<Self> {
        let selected = WarningThreshold::default();
        let this = Self::alloc(mtm).set_ivars(DiskThresholdTargetIvars {
            proxy,
            applying: Cell::new(false),
            selected: Cell::new(selected),
            slider,
            value_label,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn apply(&self, threshold: WarningThreshold) {
        self.ivars().applying.set(true);
        self.ivars().selected.set(threshold);
        self.ivars().slider.setIntegerValue(threshold.get().into());
        set_disk_threshold_value(&self.ivars().slider, &self.ivars().value_label, threshold);
        self.ivars().applying.set(false);
    }
}

pub(super) fn get_or_create_window<T>(slot: &mut Option<T>, create: impl FnOnce() -> T) -> &mut T {
    slot.get_or_insert_with(create)
}

struct PreferencesControlTargetInput {
    indicator: Retained<NSView>,
    disk_and_mole: Retained<NSView>,
    indicator_scroll: Retained<NSScrollView>,
    indicator_document: Retained<NSView>,
    indicator_area_views: IndicatorAreaViews,
    indicator_first_keys: [Retained<NSView>; 4],
    disk_and_mole_first_key: Retained<NSButton>,
    color_wells: Vec<Retained<NSColorWell>>,
}

struct PreferencesControlTargetIvars {
    state: RefCell<PreferencesAreaState>,
    indicator: Retained<NSView>,
    disk_and_mole: Retained<NSView>,
    indicator_scroll: Retained<NSScrollView>,
    indicator_document: Retained<NSView>,
    indicator_area_views: IndicatorAreaViews,
    indicator_first_keys: [Retained<NSView>; 4],
    disk_and_mole_first_key: Retained<NSButton>,
    sidebar: RefCell<Option<Retained<NSTableView>>>,
    color_wells: Vec<Retained<NSColorWell>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreferencesControlTargetIvars]
    struct PreferencesControlTarget;

    unsafe impl NSObjectProtocol for PreferencesControlTarget {}
);

impl PreferencesControlTarget {
    fn new(mtm: MainThreadMarker, input: PreferencesControlTargetInput) -> Retained<Self> {
        let PreferencesControlTargetInput {
            indicator,
            disk_and_mole,
            indicator_scroll,
            indicator_document,
            indicator_area_views,
            indicator_first_keys,
            disk_and_mole_first_key,
            color_wells,
        } = input;
        let this = Self::alloc(mtm).set_ivars(PreferencesControlTargetIvars {
            state: RefCell::new(PreferencesAreaState::new()),
            indicator,
            disk_and_mole,
            indicator_scroll,
            indicator_document,
            indicator_area_views,
            indicator_first_keys,
            disk_and_mole_first_key,
            sidebar: RefCell::new(None),
            color_wells,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn set_sidebar(&self, sidebar: Retained<NSTableView>) {
        self.ivars().sidebar.replace(Some(sidebar));
        select_visible_area(&self.ivars().state, |area| self.select_area(area));
    }

    fn select_area(&self, area: PreferencesArea) {
        let current = self.ivars().state.borrow().visible();
        let navigation = PreferencesNavigationPolicy::between(current, area);
        let state = self.ivars().state.borrow().select(area);
        self.ivars().state.replace(state);
        let visible = state.visible();
        self.ivars().indicator_area_views.set_visible_area(visible);
        self.ivars()
            .indicator
            .setHidden(visible == PreferencesArea::DiskAndMole);
        self.ivars()
            .disk_and_mole
            .setHidden(visible != PreferencesArea::DiskAndMole);
        let clip_view = self.ivars().indicator_scroll.contentView();
        let bounds = clip_view.bounds();
        let document_height = self.ivars().indicator_document.frame().size.height;
        let origin_y = navigation.scroll_origin_y(
            bounds.origin.y,
            bounds.size.height,
            document_height,
            document_height,
        );
        clip_view.scrollToPoint(NSPoint::new(bounds.origin.x, origin_y));
        self.ivars()
            .indicator_scroll
            .reflectScrolledClipView(&clip_view);
        deactivate_inactive_color_wells(
            &self.ivars().color_wells,
            |well| well.isHiddenOrHasHiddenAncestor(),
            |well| well.deactivate(),
        );
        if let Some(sidebar) = self.ivars().sidebar.borrow().as_deref() {
            unsafe {
                if let Some(index) = visible.indicator_index() {
                    sidebar.setNextKeyView(Some(&self.ivars().indicator_first_keys[index]));
                } else {
                    sidebar.setNextKeyView(Some(&self.ivars().disk_and_mole_first_key));
                }
            }
        }
    }

    fn refresh_selected_area(&self) {
        select_visible_area(&self.ivars().state, |area| self.select_area(area));
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct PreferencesSidebarDataSource;

    unsafe impl NSObjectProtocol for PreferencesSidebarDataSource {}

    unsafe impl NSTableViewDataSource for PreferencesSidebarDataSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows_in_table_view(&self, _table: &NSTableView) -> NSInteger {
            5
        }
    }
);

impl PreferencesSidebarDataSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

struct PreferencesSidebarDelegateIvars {
    target: Retained<PreferencesControlTarget>,
    table: Retained<NSTableView>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreferencesSidebarDelegateIvars]
    struct PreferencesSidebarDelegate;

    unsafe impl NSObjectProtocol for PreferencesSidebarDelegate {}
    unsafe impl NSControlTextEditingDelegate for PreferencesSidebarDelegate {}

    unsafe impl NSTableViewDelegate for PreferencesSidebarDelegate {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn table_view_view_for_table_column_row(
            &self,
            _table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            let area = PreferencesArea::from_sidebar_row(row)
                .expect("preferences sidebar requested a known destination row");
            let label = NSTextField::labelWithString(
                &objc2_foundation::NSString::from_str(area.sidebar_label()),
                MainThreadMarker::new().expect("preferences sidebar runs on the main thread"),
            );
            label.setFrame(NSRect::new(
                NSPoint::new(10.0, 4.0),
                NSSize::new(152.0, 24.0),
            ));
            label.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(
                area.sidebar_label(),
            )));
            Some(label.into_super().into_super())
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn table_view_selection_did_change(&self, _notification: &NSNotification) {
            let Some(area) = PreferencesArea::from_sidebar_row(self.ivars().table.selectedRow())
            else {
                return;
            };
            self.ivars().target.select_area(area);
        }
    }
);

impl PreferencesSidebarDelegate {
    fn new(
        mtm: MainThreadMarker,
        target: Retained<PreferencesControlTarget>,
        table: Retained<NSTableView>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PreferencesSidebarDelegateIvars { target, table });
        unsafe { msg_send![super(this), init] }
    }
}

struct PreferencesSidebar {
    _scroll: Retained<NSScrollView>,
    table: Retained<NSTableView>,
    _data_source: Retained<PreferencesSidebarDataSource>,
    _delegate: Retained<PreferencesSidebarDelegate>,
}

impl PreferencesSidebar {
    fn table(&self) -> &NSTableView {
        &self.table
    }

    fn view(&self) -> &NSScrollView {
        &self._scroll
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreferencesWindowDelegateIvars]
    struct PreferencesWindowDelegate;

    unsafe impl NSObjectProtocol for PreferencesWindowDelegate {}
    unsafe impl NSWindowDelegate for PreferencesWindowDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            for well in &self.ivars().color_wells {
                well.deactivate();
            }
            let _ = self
                .ivars()
                .proxy
                .send_event(crate::macos::RuntimeEvent::App(
                    AppEvent::PreferencesWindowClosed,
                ));
        }
    }
);

struct PreferencesWindowDelegateIvars {
    color_wells: Vec<Retained<NSColorWell>>,
    proxy: tao::event_loop::EventLoopProxy<crate::macos::RuntimeEvent>,
}

impl PreferencesWindowDelegate {
    fn new(
        mtm: MainThreadMarker,
        color_wells: Vec<Retained<NSColorWell>>,
        proxy: tao::event_loop::EventLoopProxy<crate::macos::RuntimeEvent>,
    ) -> Retained<Self> {
        let this =
            Self::alloc(mtm).set_ivars(PreferencesWindowDelegateIvars { color_wells, proxy });
        unsafe { msg_send![super(this), init] }
    }
}

struct IndicatorPage {
    root: Retained<NSView>,
    preview: PreviewPane,
    controls: IndicatorControls,
    groups_scroll: Retained<NSScrollView>,
    groups_document: Retained<NSView>,
    groups_stack: Retained<NSStackView>,
}

impl IndicatorPage {
    fn apply_preferences(
        &self,
        preferences: &IndicatorPreferences,
        cpu_icon_error: Option<&str>,
        ram_icon_error: Option<&str>,
        cpu_icon_pending: bool,
        ram_icon_pending: bool,
    ) {
        let previous_controls_height = self.controls.content_height();
        self.controls.apply(
            preferences,
            cpu_icon_error,
            ram_icon_error,
            cpu_icon_pending,
            ram_icon_pending,
        );
        let controls_height = self.controls.content_height();
        if controls_height == previous_controls_height {
            return;
        }

        let clip_view = self.groups_scroll.contentView();
        let old_bounds = clip_view.bounds();
        let old_document_height = self.groups_document.frame().size.height;
        let document_height = controls_height + 32.0;
        self.controls
            .view()
            .setFrameSize(NSSize::new(560.0, controls_height));
        self.groups_stack.setFrame(NSRect::new(
            NSPoint::new(16.0, 16.0),
            NSSize::new(580.0, controls_height),
        ));
        self.groups_document
            .setFrameSize(NSSize::new(612.0, document_height));

        let origin_y = preserve_scroll_origin_from_top(
            old_bounds.origin.y,
            old_bounds.size.height,
            old_document_height,
            document_height,
        );
        clip_view.scrollToPoint(NSPoint::new(old_bounds.origin.x, origin_y));
        self.groups_scroll.reflectScrolledClipView(&clip_view);
    }
}

struct DiskAndMolePage {
    root: Retained<NSView>,
    mole_checkbox: Retained<NSButton>,
    warning_threshold: Retained<NSSlider>,
    _warning_threshold_value: Retained<NSTextField>,
    warning_threshold_target: Retained<DiskThresholdTarget>,
}

struct PreferencesFooter {
    _indicator_view: Retained<NSView>,
    _save_recovery_view: Retained<NSView>,
    reset_all: Retained<NSButton>,
    undo: Retained<NSButton>,
    save_error: Retained<NSTextField>,
    retry_save: Retained<NSButton>,
}

impl PreferencesFooter {
    fn apply(&self, presentation: PreferencesShellPresentation) {
        self.reset_all
            .setHidden(!presentation.indicator_reset_visible());
        self.reset_all
            .setEnabled(presentation.indicator_reset_visible());
        self.undo.setHidden(!presentation.undo_visible());
        self.undo.setEnabled(presentation.undo_visible());
        self.retry_save.setHidden(!presentation.retry_visible());
        self.retry_save.setEnabled(presentation.retry_visible());
        self.save_error
            .setStringValue(&objc2_foundation::NSString::from_str(
                presentation.save_error().unwrap_or(""),
            ));
        self.save_error.setAccessibilityLabel(
            presentation
                .save_error()
                .map(objc2_foundation::NSString::from_str)
                .as_deref(),
        );
        self.save_error
            .setHidden(presentation.save_error().is_none());
    }
}

pub(super) struct PreferencesWindow {
    pub(super) window: Retained<NSWindow>,
    _host: Retained<PreferencesWindowHost>,
    _sidebar: PreferencesSidebar,
    indicator: IndicatorPage,
    disk_and_mole: DiskAndMolePage,
    _footer: PreferencesFooter,
    controls_cache: RefCell<PreferencesControlsCache>,
    _area_target: Retained<PreferencesControlTarget>,
    _delegate: Retained<PreferencesWindowDelegate>,
}

impl PreferencesWindow {
    pub(super) fn new(
        mtm: MainThreadMarker,
        target: &ControlTarget,
        presentation: RuntimePresentation,
        icon_asset_store: IconAssetStore,
    ) -> Self {
        let contract = PreferencesShellContract::new();
        let (width, height) = contract.content_size();
        let host = PreferencesWindowHost::new(
            mtm,
            &presentation.window_title("Preferências do Statlet"),
            NSSize::new(width, height),
            target.event_proxy(),
        );
        let window = Retained::into_super(host.clone());
        let content = window
            .contentView()
            .expect("preferences window content view");

        let indicator = create_indicator_page(
            mtm,
            contract,
            target.event_proxy(),
            presentation,
            icon_asset_store,
        );
        let disk_and_mole = create_disk_and_mole_page(mtm, contract, target);
        disk_and_mole.root.setHidden(true);
        content.addSubview(&indicator.root);
        content.addSubview(&disk_and_mole.root);

        let area_target = PreferencesControlTarget::new(
            mtm,
            PreferencesControlTargetInput {
                indicator: indicator.root.clone(),
                disk_and_mole: disk_and_mole.root.clone(),
                indicator_scroll: indicator.groups_scroll.clone(),
                indicator_document: indicator.groups_document.clone(),
                indicator_area_views: indicator.controls.area_views(),
                indicator_first_keys: [
                    indicator
                        .controls
                        .first_key_view_for(PreferencesArea::Colors)
                        .retain(),
                    indicator
                        .controls
                        .first_key_view_for(PreferencesArea::Labels)
                        .retain(),
                    indicator
                        .controls
                        .first_key_view_for(PreferencesArea::Typography)
                        .retain(),
                    indicator
                        .controls
                        .first_key_view_for(PreferencesArea::Refresh)
                        .retain(),
                ],
                disk_and_mole_first_key: disk_and_mole.mole_checkbox.clone(),
                color_wells: indicator.controls.wells(),
            },
        );
        let sidebar = create_preferences_sidebar(mtm, contract, area_target.clone());
        area_target.set_sidebar(sidebar.table.clone());
        content.addSubview(sidebar.view());

        let footer = create_footer(mtm, contract, &indicator.root, &content, target);
        unsafe {
            sidebar.table().setNextKeyView(Some(
                indicator
                    .controls
                    .first_key_view_for(PreferencesArea::Colors),
            ));
        }
        window.setInitialFirstResponder(Some(sidebar.table()));
        let delegate =
            PreferencesWindowDelegate::new(mtm, indicator.controls.wells(), target.event_proxy());
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        Self {
            window,
            _host: host,
            _sidebar: sidebar,
            indicator,
            disk_and_mole,
            _footer: footer,
            controls_cache: RefCell::new(PreferencesControlsCache::default()),
            _area_target: area_target,
            _delegate: delegate,
        }
    }

    pub(super) fn apply(&self, state: &AppState, _previews: Option<&PreviewImages>) {
        if !self.controls_cache.borrow_mut().should_apply(state) {
            return;
        }

        self.disk_and_mole
            .mole_checkbox
            .setState(if state.preferences.mole_integration_enabled {
                NSControlStateValueOn
            } else {
                0
            });
        self.disk_and_mole
            .warning_threshold_target
            .apply(state.preferences.warning_threshold);
        self.disk_and_mole
            .warning_threshold
            .setEnabled(state.preferences.mole_integration_enabled);
        self.indicator.apply_preferences(
            &state.preferences.indicator,
            state.indicator_icon_error(statlet::indicator_preferences::MetricKind::Cpu),
            state.indicator_icon_error(statlet::indicator_preferences::MetricKind::Ram),
            state.indicator_icon_pending(statlet::indicator_preferences::MetricKind::Cpu),
            state.indicator_icon_pending(statlet::indicator_preferences::MetricKind::Ram),
        );
        self._area_target.refresh_selected_area();
        let footer = PreferencesShellPresentation::new(
            PreferencesArea::Colors,
            state.can_undo_indicator_reset,
            state.preferences_save_status,
        );
        self._footer.apply(footer);
        self._host
            .set_can_undo_indicator_reset(state.can_undo_indicator_reset);
        unsafe {
            for last in self.indicator.controls.last_key_views() {
                last.setNextKeyView(Some(&self._footer.reset_all));
            }
            self.disk_and_mole
                .mole_checkbox
                .setNextKeyView(Some(&self.disk_and_mole.warning_threshold));
            self.disk_and_mole
                .warning_threshold
                .setNextKeyView(Some(self._sidebar.table()));
            if footer.undo_visible() {
                self._footer
                    .reset_all
                    .setNextKeyView(Some(&self._footer.undo));
            } else if footer.retry_visible() {
                self._footer
                    .reset_all
                    .setNextKeyView(Some(&self._footer.retry_save));
            } else {
                self._footer
                    .reset_all
                    .setNextKeyView(Some(self._sidebar.table()));
            }
            if footer.retry_visible() {
                self._footer
                    .undo
                    .setNextKeyView(Some(&self._footer.retry_save));
            } else {
                self._footer
                    .undo
                    .setNextKeyView(Some(self._sidebar.table()));
            }
            self._footer
                .retry_save
                .setNextKeyView(Some(self._sidebar.table()));
            let disk_shell = PreferencesShellPresentation::new(
                PreferencesArea::DiskAndMole,
                state.can_undo_indicator_reset,
                state.preferences_save_status,
            );
            match disk_shell.focus_target_after_area_controls() {
                PreferencesShellFocusTarget::RetrySave => self
                    .disk_and_mole
                    .warning_threshold
                    .setNextKeyView(Some(&self._footer.retry_save)),
                PreferencesShellFocusTarget::Sidebar => self
                    .disk_and_mole
                    .warning_threshold
                    .setNextKeyView(Some(self._sidebar.table())),
                PreferencesShellFocusTarget::ResetIndicator => {
                    unreachable!("Disk and Mole never focuses indicator reset")
                }
            }
        }
    }

    pub(super) fn apply_surfaces(&self, surfaces: IndicatorSurfaceUpdate) {
        let IndicatorSurfaceUpdate {
            previews,
            font_fallback,
            contrast_warnings,
            summaries,
            layout,
            environment,
        } = surfaces;
        self.indicator.preview.apply_with_contrast(
            &previews,
            &layout.light,
            font_fallback.as_ref(),
            &environment,
            contrast_warnings,
            &summaries,
        );
        self.indicator
            .controls
            .apply_diagnostics(font_fallback.as_ref(), &layout);
    }

    pub(super) fn is_created_and_visible(&self) -> bool {
        self.window.isVisible()
    }
}

fn create_indicator_page(
    mtm: MainThreadMarker,
    contract: PreferencesShellContract,
    proxy: tao::event_loop::EventLoopProxy<crate::macos::RuntimeEvent>,
    presentation: RuntimePresentation,
    icon_asset_store: IconAssetStore,
) -> IndicatorPage {
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(contract.sidebar_width(), 0.0),
            NSSize::new(680.0, 640.0),
        ),
    );

    let identifiers = contract.accessibility_identifiers();
    let preview = PreviewPane::new(
        mtm,
        fixed_region_frame(
            contract,
            PreferencesRegion::Previews,
            NSRect::new(NSPoint::new(24.0, 462.0), NSSize::new(632.0, 150.0)),
        ),
        [identifiers[1], identifiers[2]],
    );
    root.addSubview(preview.view());

    let groups_scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(NSPoint::new(24.0, 96.0), NSSize::new(632.0, 344.0)),
    );
    groups_scroll.setHasVerticalScroller(matches!(
        contract.placement(PreferencesRegion::IndicatorGroups),
        RegionPlacement::Scrollable
    ));
    groups_scroll.setDrawsBackground(false);
    groups_scroll.setAccessibilityLabel(Some(ns_string!("Grupos de preferências do indicador")));
    let controls = IndicatorControls::new(
        mtm,
        proxy,
        presentation,
        ThumbnailAssetPlan::new(icon_asset_store),
    );
    let controls_height = controls.content_height();
    let groups_document = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(612.0, controls_height + 32.0),
        ),
    );
    let groups_stack = NSStackView::initWithFrame(
        NSStackView::alloc(mtm),
        NSRect::new(
            NSPoint::new(16.0, 16.0),
            NSSize::new(580.0, controls_height),
        ),
    );
    let _: () = unsafe { msg_send![&*groups_stack, setOrientation: 1isize] };
    groups_stack.setSpacing(24.0);
    groups_stack.addSubview(controls.view());
    groups_document.addSubview(&groups_stack);
    groups_scroll.setDocumentView(Some(&groups_document));
    root.addSubview(&groups_scroll);

    IndicatorPage {
        root,
        preview,
        controls,
        groups_scroll,
        groups_document,
        groups_stack,
    }
}

fn create_footer(
    mtm: MainThreadMarker,
    contract: PreferencesShellContract,
    indicator_root: &NSView,
    content: &NSView,
    target: &ControlTarget,
) -> PreferencesFooter {
    let save_error_layout = MessageLayout::preferences_save_error();
    let indicator_view = NSView::initWithFrame(
        NSView::alloc(mtm),
        fixed_region_frame(
            contract,
            PreferencesRegion::Footer,
            NSRect::new(NSPoint::new(24.0, 20.0), NSSize::new(414.0, 56.0)),
        ),
    );
    let save_recovery_view = NSView::initWithFrame(
        NSView::alloc(mtm),
        fixed_region_frame(
            contract,
            PreferencesRegion::Footer,
            NSRect::new(
                NSPoint::new(contract.sidebar_width() + 444.0, 10.0),
                NSSize::new(212.0, save_error_layout.height() + 30.0),
            ),
        ),
    );
    let ids = contract.accessibility_identifiers();
    let reset_all = footer_button(
        mtm,
        "Restaurar indicador aos padrões…",
        ids[3],
        NSRect::new(NSPoint::new(0.0, 10.0), NSSize::new(230.0, 34.0)),
        target,
        sel!(resetIndicator:),
    );
    let undo = footer_button(
        mtm,
        "Desfazer restauração",
        ids[4],
        NSRect::new(NSPoint::new(244.0, 10.0), NSSize::new(170.0, 34.0)),
        target,
        sel!(undoIndicatorReset:),
    );
    let save_error = NSTextField::labelWithString(ns_string!(""), mtm);
    save_error.setFrame(NSRect::new(
        NSPoint::new(save_error_layout.x(), 30.0),
        NSSize::new(save_error_layout.width(), save_error_layout.height()),
    ));
    save_error.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByWordWrapping);
    save_error.setMaximumNumberOfLines(save_error_layout.maximum_lines());
    save_error.setTextColor(Some(&objc2_app_kit::NSColor::systemRedColor()));
    save_error.setHidden(true);
    let retry_save = footer_button(
        mtm,
        "Tentar novamente",
        ids[5],
        NSRect::new(NSPoint::new(28.0, 0.0), NSSize::new(164.0, 30.0)),
        target,
        sel!(retrySavePreferences:),
    );
    for button in [&reset_all, &undo] {
        indicator_view.addSubview(button);
    }
    save_recovery_view.addSubview(&retry_save);
    save_recovery_view.addSubview(&save_error);
    indicator_root.addSubview(&indicator_view);
    content.addSubview(&save_recovery_view);
    let footer = PreferencesFooter {
        _indicator_view: indicator_view,
        _save_recovery_view: save_recovery_view,
        reset_all,
        undo,
        save_error,
        retry_save,
    };
    footer.apply(PreferencesShellPresentation::new(
        PreferencesArea::Colors,
        false,
        statlet::core::PreferencesSaveStatus::Saved,
    ));
    footer
}

fn fixed_region_frame(
    contract: PreferencesShellContract,
    region: PreferencesRegion,
    frame: NSRect,
) -> NSRect {
    match contract.placement(region) {
        RegionPlacement::Fixed => frame,
        RegionPlacement::Scrollable => panic!("fixed preferences region cannot scroll"),
    }
}

fn footer_button(
    mtm: MainThreadMarker,
    title: &str,
    identifier: &str,
    frame: NSRect,
    target: &ControlTarget,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &objc2_foundation::NSString::from_str(title),
            Some(target as &AnyObject),
            Some(action),
            mtm,
        )
    };
    button.setFrame(frame);
    button.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(identifier)));
    button
}

fn create_preferences_sidebar(
    mtm: MainThreadMarker,
    contract: PreferencesShellContract,
    target: Retained<PreferencesControlTarget>,
) -> PreferencesSidebar {
    let (_, height) = contract.content_size();
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(contract.sidebar_width(), height),
        ),
    );
    scroll.setHasVerticalScroller(false);
    scroll.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
        contract.accessibility_identifiers()[0],
    )));
    scroll.setAccessibilityLabel(Some(ns_string!("Áreas de preferências")));

    let table = NSTableView::initWithFrame(
        NSTableView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(contract.sidebar_width(), height),
        ),
    );
    table.setStyle(NSTableViewStyle::SourceList);
    table.setAllowsMultipleSelection(false);
    table.setAllowsEmptySelection(false);
    table.setRowHeight(32.0);
    if !contract.sidebar_header_visible() {
        table.setHeaderView(None);
    }
    table.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
        contract.sidebar_accessibility_identifier(),
    )));
    table.setAccessibilityLabel(Some(ns_string!("Áreas de preferências")));
    let column = NSTableColumn::initWithIdentifier(
        NSTableColumn::alloc(mtm),
        &NSUserInterfaceItemIdentifier::from_str("preferences.sidebar.destination"),
    );
    column.setWidth(contract.sidebar_width());
    table.addTableColumn(&column);

    let data_source = PreferencesSidebarDataSource::new(mtm);
    let delegate = PreferencesSidebarDelegate::new(mtm, target, table.clone());
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&*data_source)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    }
    table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(0), false);
    scroll.setDocumentView(Some(&table));

    PreferencesSidebar {
        _scroll: scroll,
        table,
        _data_source: data_source,
        _delegate: delegate,
    }
}

fn create_disk_and_mole_page(
    mtm: MainThreadMarker,
    contract: PreferencesShellContract,
    target: &ControlTarget,
) -> DiskAndMolePage {
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(contract.sidebar_width(), 0.0),
            NSSize::new(680.0, 640.0),
        ),
    );
    let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(100.0, 190.0), NSSize::new(480.0, 238.0)),
    );

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

    let threshold = NSSlider::initWithFrame(
        NSSlider::alloc(mtm),
        NSRect::new(NSPoint::new(245.0, 66.0), NSSize::new(165.0, 30.0)),
    );
    configure_discrete_slider(&threshold, disk_threshold_slider_contract());
    let threshold_value = NSTextField::labelWithString(ns_string!("90%"), mtm);
    threshold_value.setFrame(NSRect::new(
        NSPoint::new(420.0, 66.0),
        NSSize::new(54.0, 30.0),
    ));
    threshold_value.setAccessibilityIdentifier(Some(ns_string!("disk.warning.threshold.value")));
    let threshold_target = DiskThresholdTarget::new(
        mtm,
        target.event_proxy(),
        threshold.clone(),
        threshold_value.clone(),
    );
    unsafe {
        threshold.setTarget(Some(&*threshold_target as &AnyObject));
        threshold.setAction(Some(sel!(changeWarningThreshold:)));
    }
    threshold_target.apply(WarningThreshold::default());

    content.addSubview(&heading);
    content.addSubview(&checkbox);
    content.addSubview(&explanation);
    content.addSubview(&threshold_label);
    content.addSubview(&threshold);
    content.addSubview(&threshold_value);
    root.addSubview(&content);

    DiskAndMolePage {
        root,
        mole_checkbox: checkbox,
        warning_threshold: threshold,
        _warning_threshold_value: threshold_value,
        warning_threshold_target: threshold_target,
    }
}

#[cfg(test)]
mod tests {
    use statlet::core::WarningThreshold;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::super::common::should_intercept_indicator_undo;
    use super::{
        deactivate_inactive_color_wells, disk_threshold_slider_contract, font_size_slider_contract,
        get_or_create_window, label_spacing_slider_contract, select_visible_area,
        system_symbol_size_slider_contract, warning_threshold_from_slider_value, PreferencesArea,
        PreferencesAreaState, PreferencesRegion, PreferencesShellContract, RegionPlacement,
    };
    use statlet::core::PreferencesSaveStatus;
    use statlet::preferences_view::PreferencesShellPresentation;

    #[test]
    fn selecting_labels_deactivates_hidden_color_wells_without_disconnecting_visible_label_wells() {
        let hidden_or_has_hidden_ancestor = [true, true, false];
        let mut deactivated = Vec::new();

        deactivate_inactive_color_wells(
            &hidden_or_has_hidden_ancestor,
            |hidden| *hidden,
            |hidden| deactivated.push(*hidden),
        );

        assert_eq!(deactivated, [true, true]);
    }

    #[test]
    fn discrete_slider_contracts_match_the_approved_domain_ranges() {
        let spacing = label_spacing_slider_contract();
        assert_eq!(
            (
                spacing.min(),
                spacing.max(),
                spacing.step(),
                spacing.ticks()
            ),
            (0, 4, 1, 5)
        );
        assert_eq!(spacing.identifier(), "indicator.labels.spacing");

        let font = font_size_slider_contract();
        assert_eq!(
            (font.min(), font.max(), font.step(), font.ticks()),
            (9, 14, 1, 6)
        );
        assert_eq!(font.identifier(), "indicator.font.size");

        let symbol = system_symbol_size_slider_contract();
        assert_eq!(
            (symbol.min(), symbol.max(), symbol.step(), symbol.ticks()),
            (8, 14, 1, 7)
        );
        assert_eq!(
            symbol.identifier(),
            "indicator.identifiers.system-symbol-size"
        );

        let disk = disk_threshold_slider_contract();
        assert_eq!(
            (disk.min(), disk.max(), disk.step(), disk.ticks()),
            (70, 95, 5, 6)
        );
        assert_eq!(disk.identifier(), "disk.warning.threshold");

        for contract in [spacing, font, symbol, disk] {
            assert!(contract.continuous());
            assert!(contract.tick_values_only());
            assert!(!contract.accessibility_label().is_empty());
            assert!(!contract.accessibility_help().is_empty());
        }
    }

    #[test]
    fn disk_slider_values_cross_the_public_warning_threshold_seam() {
        for value in [70, 75, 80, 85, 90, 95] {
            assert_eq!(
                warning_threshold_from_slider_value(value).map(WarningThreshold::get),
                Some(u8::try_from(value).unwrap())
            );
        }

        for value in [69, 71, 94, 96] {
            assert_eq!(warning_threshold_from_slider_value(value), None);
        }
    }

    #[test]
    fn setting_the_sidebar_releases_the_visible_area_borrow_before_selection() {
        let state = RefCell::new(PreferencesAreaState::new());

        select_visible_area(&state, |area| {
            let next = state.borrow().select(area);
            state.replace(next);
        });

        assert_eq!(state.borrow().visible(), PreferencesArea::Colors);
    }

    #[test]
    fn selecting_an_area_shows_exactly_one_preferences_page() {
        let areas = [
            PreferencesArea::Colors,
            PreferencesArea::Labels,
            PreferencesArea::Typography,
            PreferencesArea::Refresh,
            PreferencesArea::DiskAndMole,
        ];
        let state = PreferencesAreaState::new();

        assert_eq!(state.visible(), PreferencesArea::Colors);
        assert_eq!(
            areas
                .iter()
                .filter(|area| state.visible() == **area)
                .count(),
            1
        );

        let state = state.select(PreferencesArea::DiskAndMole);
        assert_eq!(state.visible(), PreferencesArea::DiskAndMole);
        assert_eq!(
            areas
                .iter()
                .filter(|area| state.visible() == **area)
                .count(),
            1
        );
    }

    #[test]
    fn preferences_shell_contract_has_exact_size_and_sidebar_labels() {
        let contract = PreferencesShellContract::new();

        assert_eq!(contract.content_size(), (860.0, 700.0));
        assert_eq!(PreferencesArea::Colors.sidebar_label(), "Cores");
        assert_eq!(PreferencesArea::Labels.sidebar_label(), "Rótulos");
        assert_eq!(PreferencesArea::Typography.sidebar_label(), "Tipografia");
        assert_eq!(PreferencesArea::Refresh.sidebar_label(), "Atualização");
        assert_eq!(PreferencesArea::DiskAndMole.sidebar_label(), "Disco e Mole");
    }

    #[test]
    fn preferences_shell_contract_exposes_a_stable_keyboard_navigable_sidebar() {
        let contract = PreferencesShellContract::new();

        assert_eq!(contract.content_size(), (860.0, 700.0));
        assert_eq!(contract.sidebar_width(), 180.0);
        assert!(!contract.sidebar_header_visible());
        assert_eq!(PreferencesArea::Colors.sidebar_label(), "Cores");
        assert_eq!(PreferencesArea::Labels.sidebar_label(), "Rótulos");
        assert_eq!(PreferencesArea::Typography.sidebar_label(), "Tipografia");
        assert_eq!(PreferencesArea::Refresh.sidebar_label(), "Atualização");
        assert_eq!(PreferencesArea::DiskAndMole.sidebar_label(), "Disco e Mole");
        assert_eq!(
            contract.sidebar_accessibility_identifier(),
            "preferences.sidebar"
        );
        assert_eq!(
            PreferencesArea::from_sidebar_row(0),
            Some(PreferencesArea::Colors)
        );
        assert_eq!(
            PreferencesArea::from_sidebar_row(1),
            Some(PreferencesArea::Labels)
        );
        assert_eq!(
            PreferencesArea::from_sidebar_row(2),
            Some(PreferencesArea::Typography)
        );
        assert_eq!(
            PreferencesArea::from_sidebar_row(3),
            Some(PreferencesArea::Refresh)
        );
        assert_eq!(
            PreferencesArea::from_sidebar_row(4),
            Some(PreferencesArea::DiskAndMole)
        );
        assert_eq!(PreferencesArea::from_sidebar_row(5), None);
    }

    #[test]
    fn preferences_shell_contract_keeps_previews_and_footer_outside_the_scroll_region() {
        let contract = PreferencesShellContract::new();

        assert_eq!(
            contract.placement(PreferencesRegion::IndicatorGroups),
            RegionPlacement::Scrollable
        );
        assert_eq!(
            contract.placement(PreferencesRegion::Previews),
            RegionPlacement::Fixed
        );
        assert_eq!(
            contract.placement(PreferencesRegion::Footer),
            RegionPlacement::Fixed
        );
    }

    #[test]
    fn preferences_shell_contract_exposes_stable_accessibility_seams() {
        let contract = PreferencesShellContract::new();

        assert_eq!(
            contract.accessibility_identifiers(),
            [
                "preferences.area",
                "indicator.preview.light",
                "indicator.preview.dark",
                "indicator.reset.all",
                "indicator.reset.undo",
                "indicator.save.retry",
            ]
        );
        assert_eq!(
            contract.identifier_accessibility_identifiers(),
            [
                "indicator.cpu.identifier.mode",
                "indicator.cpu.identifier.symbol",
                "indicator.cpu.identifier.choose-png",
                "indicator.cpu.identifier.thumbnail",
                "indicator.cpu.identifier.status",
                "indicator.cpu.identifier.remove",
                "indicator.ram.identifier.mode",
                "indicator.ram.identifier.symbol",
                "indicator.ram.identifier.choose-png",
                "indicator.ram.identifier.thumbnail",
                "indicator.ram.identifier.status",
                "indicator.ram.identifier.remove",
            ]
        );
    }

    #[test]
    fn get_or_create_window_reuses_the_retained_instance() {
        let creations = Cell::new(0);
        let mut slot = None;
        let first = get_or_create_window(&mut slot, || {
            creations.set(creations.get() + 1);
            Rc::new(())
        })
        .clone();
        let second = get_or_create_window(&mut slot, || {
            creations.set(creations.get() + 1);
            Rc::new(())
        })
        .clone();

        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(creations.get(), 1);
    }

    #[test]
    fn footer_presentation_exposes_only_available_recovery_actions() {
        let normal = PreferencesShellPresentation::new(
            PreferencesArea::Colors,
            false,
            PreferencesSaveStatus::Saved,
        );
        assert!(!normal.undo_visible());
        assert!(!normal.retry_visible());
        assert_eq!(normal.save_error(), None);

        let recovery = PreferencesShellPresentation::new(
            PreferencesArea::Colors,
            true,
            PreferencesSaveStatus::Failed,
        );
        assert!(recovery.undo_visible());
        assert!(recovery.retry_visible());
        assert_eq!(
            recovery.save_error(),
            Some("Não foi possível salvar as preferências.")
        );
    }

    #[test]
    fn command_z_is_reserved_only_for_an_available_indicator_reset_undo() {
        assert!(should_intercept_indicator_undo("z", true, true));
        assert!(!should_intercept_indicator_undo("z", true, false));
        assert!(!should_intercept_indicator_undo("z", false, true));
        assert!(!should_intercept_indicator_undo("Z", true, true));
    }
}
