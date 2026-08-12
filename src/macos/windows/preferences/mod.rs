use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSControlStateValueOn, NSImageView, NSLayoutConstraint,
    NSPopUpButton, NSScrollView, NSSegmentSwitchTracking, NSSegmentedControl, NSStackView,
    NSTextField, NSView, NSWindow, NSWindowDelegate,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};
use statlet::core::{AppState, WarningThreshold};

use super::common::{create_window, threshold_title, ControlTarget};
use super::{
    IndicatorFontFallback, IndicatorLayoutDiagnostics, IndicatorSurfaceUpdate,
    PreviewContrastWarnings,
};
use crate::macos::environment::VisualEnvironment;
use crate::macos::renderer::PreviewImages;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum PreferencesArea {
    #[default]
    Indicator,
    DiskAndMole,
}

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

    pub(super) fn is_visible(self, area: PreferencesArea) -> bool {
        self.visible == area
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
        (680.0, 700.0)
    }

    pub(super) const fn selector_labels(self) -> [&'static str; 2] {
        ["Indicador", "Disco e Mole"]
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

    pub(super) const fn preview_images_are_accessibility_elements(self) -> bool {
        false
    }

    pub(super) const fn footer_placeholders_are_hidden_and_disabled(self) -> bool {
        true
    }
}

pub(super) fn get_or_create_window<T>(slot: &mut Option<T>, create: impl FnOnce() -> T) -> &mut T {
    slot.get_or_insert_with(create)
}

struct PreferencesControlTargetIvars {
    state: RefCell<PreferencesAreaState>,
    indicator: Retained<NSView>,
    disk_and_mole: Retained<NSView>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreferencesControlTargetIvars]
    struct PreferencesControlTarget;

    unsafe impl NSObjectProtocol for PreferencesControlTarget {}

    impl PreferencesControlTarget {
        #[unsafe(method(changePreferencesArea:))]
        fn change_preferences_area(&self, sender: &NSSegmentedControl) {
            let area = if sender.selectedSegment() == 1 {
                PreferencesArea::DiskAndMole
            } else {
                PreferencesArea::Indicator
            };
            let state = self.ivars().state.borrow().select(area);
            self.ivars().state.replace(state);
            let visible = state.visible();
            self.ivars()
                .indicator
                .setHidden(!state.is_visible(PreferencesArea::Indicator));
            self.ivars()
                .disk_and_mole
                .setHidden(visible != PreferencesArea::DiskAndMole);
        }
    }
);

impl PreferencesControlTarget {
    fn new(
        mtm: MainThreadMarker,
        indicator: Retained<NSView>,
        disk_and_mole: Retained<NSView>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PreferencesControlTargetIvars {
            state: RefCell::new(PreferencesAreaState::new()),
            indicator,
            disk_and_mole,
        });
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct PreferencesWindowDelegate;

    unsafe impl NSObjectProtocol for PreferencesWindowDelegate {}
    unsafe impl NSWindowDelegate for PreferencesWindowDelegate {}
);

impl PreferencesWindowDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

struct IndicatorPage {
    root: Retained<NSView>,
    light_image: Retained<NSImageView>,
    dark_image: Retained<NSImageView>,
    light_description: Retained<NSTextField>,
    dark_description: Retained<NSTextField>,
    _groups_scroll: Retained<NSScrollView>,
    _groups_stack: Retained<NSStackView>,
    _preview_stack: Retained<NSStackView>,
}

struct DiskAndMolePage {
    root: Retained<NSView>,
    mole_checkbox: Retained<NSButton>,
    warning_threshold: Retained<NSPopUpButton>,
}

struct PreferencesFooter {
    _view: Retained<NSView>,
    _reset_all: Retained<NSButton>,
    _undo: Retained<NSButton>,
    _retry_save: Retained<NSButton>,
}

pub(super) struct PreferencesWindow {
    pub(super) window: Retained<NSWindow>,
    _area_selector: Retained<NSSegmentedControl>,
    indicator: IndicatorPage,
    disk_and_mole: DiskAndMolePage,
    _footer: PreferencesFooter,
    _area_target: Retained<PreferencesControlTarget>,
    _delegate: Retained<PreferencesWindowDelegate>,
    indicator_previews: RefCell<Option<PreviewImages>>,
    indicator_font_fallback: RefCell<Option<IndicatorFontFallback>>,
    indicator_contrast_warnings: RefCell<Option<PreviewContrastWarnings>>,
    indicator_layout: RefCell<Option<IndicatorLayoutDiagnostics>>,
    visual_environment: RefCell<Option<VisualEnvironment>>,
}

impl PreferencesWindow {
    pub(super) fn new(mtm: MainThreadMarker, target: &ControlTarget) -> Self {
        let contract = PreferencesShellContract::new();
        let (width, height) = contract.content_size();
        let window = create_window(mtm, "Preferências do Statlet", NSSize::new(width, height));
        let content = window
            .contentView()
            .expect("preferences window content view");

        let indicator = create_indicator_page(mtm, contract);
        let disk_and_mole = create_disk_and_mole_page(mtm, target);
        disk_and_mole.root.setHidden(true);
        content.addSubview(&indicator.root);
        content.addSubview(&disk_and_mole.root);

        let area_target =
            PreferencesControlTarget::new(mtm, indicator.root.clone(), disk_and_mole.root.clone());
        let labels = contract.selector_labels();
        let label_strings = [
            objc2_foundation::NSString::from_str(labels[0]),
            objc2_foundation::NSString::from_str(labels[1]),
        ];
        let label_refs = [&*label_strings[0], &*label_strings[1]];
        let area_selector = unsafe {
            NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
                &NSArray::from_slice(&label_refs),
                NSSegmentSwitchTracking::SelectOne,
                Some(&*area_target as &AnyObject),
                Some(sel!(changePreferencesArea:)),
                mtm,
            )
        };
        area_selector.setSelectedSegment(0);
        area_selector.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            contract.accessibility_identifiers()[0],
        )));
        area_selector.setTranslatesAutoresizingMaskIntoConstraints(false);
        content.addSubview(&area_selector);
        let constraints = NSArray::from_retained_slice(&[
            area_selector
                .centerXAnchor()
                .constraintEqualToAnchor(&content.centerXAnchor()),
            area_selector
                .topAnchor()
                .constraintEqualToAnchor_constant(&content.topAnchor(), -20.0),
            area_selector.widthAnchor().constraintEqualToConstant(260.0),
            area_selector.heightAnchor().constraintEqualToConstant(28.0),
        ]);
        NSLayoutConstraint::activateConstraints(&constraints);

        let footer = create_footer(mtm, contract, &indicator.root);
        window.setInitialFirstResponder(Some(&area_selector));
        let delegate = PreferencesWindowDelegate::new(mtm);
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        Self {
            window,
            _area_selector: area_selector,
            indicator,
            disk_and_mole,
            _footer: footer,
            _area_target: area_target,
            _delegate: delegate,
            indicator_previews: RefCell::new(None),
            indicator_font_fallback: RefCell::new(None),
            indicator_contrast_warnings: RefCell::new(None),
            indicator_layout: RefCell::new(None),
            visual_environment: RefCell::new(None),
        }
    }

    pub(super) fn apply(&self, state: &AppState, previews: Option<&PreviewImages>) {
        self.disk_and_mole
            .mole_checkbox
            .setState(if state.preferences.mole_integration_enabled {
                NSControlStateValueOn
            } else {
                0
            });
        self.disk_and_mole
            .warning_threshold
            .selectItemWithTitle(&threshold_title(state.preferences.warning_threshold));
        self.disk_and_mole
            .warning_threshold
            .setEnabled(state.preferences.mole_integration_enabled);
        if let Some(previews) = previews {
            self.set_preview_images(previews);
        }
    }

    pub(super) fn apply_surfaces(&self, surfaces: IndicatorSurfaceUpdate) {
        let IndicatorSurfaceUpdate {
            previews,
            font_fallback,
            contrast_warnings,
            layout,
            environment,
        } = surfaces;
        self.set_preview_images(&previews);
        self.update_preview_description(true, contrast_warnings.light);
        self.update_preview_description(false, contrast_warnings.dark);
        self.indicator_previews.replace(Some(previews));
        self.indicator_font_fallback.replace(font_fallback);
        self.indicator_contrast_warnings
            .replace(Some(contrast_warnings));
        self.indicator_layout.replace(Some(layout));
        self.visual_environment.replace(Some(environment));
    }

    pub(super) fn is_created_and_visible(&self) -> bool {
        self.window.isVisible()
    }

    fn set_preview_images(&self, previews: &PreviewImages) {
        self.indicator.light_image.setImage(Some(&previews.light));
        self.indicator.dark_image.setImage(Some(&previews.dark));
    }

    fn update_preview_description(&self, light: bool, contrast_warning: bool) {
        let appearance = if light { "clara" } else { "escura" };
        let description = if contrast_warning {
            format!(
                "Prévia do indicador em aparência {appearance}. Aviso: o contraste pode ser insuficiente."
            )
        } else {
            format!("Prévia do indicador em aparência {appearance}.")
        };
        let field = if light {
            &self.indicator.light_description
        } else {
            &self.indicator.dark_description
        };
        field.setStringValue(&objc2_foundation::NSString::from_str(&description));
        field.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&description)));
    }
}

fn create_indicator_page(
    mtm: MainThreadMarker,
    contract: PreferencesShellContract,
) -> IndicatorPage {
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(680.0, 640.0)),
    );

    let preview_stack = NSStackView::initWithFrame(
        NSStackView::alloc(mtm),
        fixed_region_frame(
            contract,
            PreferencesRegion::Previews,
            NSRect::new(NSPoint::new(24.0, 462.0), NSSize::new(632.0, 150.0)),
        ),
    );
    preview_stack.setSpacing(16.0);
    let light_card = create_preview_card(mtm, "Claro");
    let dark_card = create_preview_card(mtm, "Escuro");
    preview_stack.addArrangedSubview(&light_card);
    preview_stack.addArrangedSubview(&dark_card);
    root.addSubview(&preview_stack);

    let light_image = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        NSRect::new(NSPoint::new(20.0, 50.0), NSSize::new(270.0, 44.0)),
    );
    let dark_image = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        NSRect::new(NSPoint::new(20.0, 50.0), NSSize::new(270.0, 44.0)),
    );
    light_image.setAccessibilityElement(contract.preview_images_are_accessibility_elements());
    dark_image.setAccessibilityElement(contract.preview_images_are_accessibility_elements());
    light_card.addSubview(&light_image);
    dark_card.addSubview(&dark_image);

    let identifiers = contract.accessibility_identifiers();
    let light_description = preview_description(
        mtm,
        "Prévia do indicador em aparência clara.",
        identifiers[1],
    );
    let dark_description = preview_description(
        mtm,
        "Prévia do indicador em aparência escura.",
        identifiers[2],
    );
    light_card.addSubview(&light_description);
    dark_card.addSubview(&dark_description);

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
    let groups_document = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(612.0, 520.0)),
    );
    let groups_stack = NSStackView::initWithFrame(
        NSStackView::alloc(mtm),
        NSRect::new(NSPoint::new(16.0, 16.0), NSSize::new(580.0, 488.0)),
    );
    let _: () = unsafe { msg_send![&*groups_stack, setOrientation: 1isize] };
    groups_stack.setSpacing(24.0);
    for title in ["CPU e RAM", "Rótulos", "Tipografia", "Atualização"] {
        let heading =
            NSTextField::labelWithString(&objc2_foundation::NSString::from_str(title), mtm);
        heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(15.0)));
        groups_stack.addArrangedSubview(&heading);
    }
    groups_document.addSubview(&groups_stack);
    groups_scroll.setDocumentView(Some(&groups_document));
    root.addSubview(&groups_scroll);

    IndicatorPage {
        root,
        light_image,
        dark_image,
        light_description,
        dark_description,
        _groups_scroll: groups_scroll,
        _groups_stack: groups_stack,
        _preview_stack: preview_stack,
    }
}

fn create_preview_card(mtm: MainThreadMarker, title: &str) -> Retained<NSView> {
    let card = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(308.0, 150.0)),
    );
    let heading = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(title), mtm);
    heading.setFrame(NSRect::new(
        NSPoint::new(16.0, 116.0),
        NSSize::new(276.0, 22.0),
    ));
    heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(13.0)));
    card.addSubview(&heading);
    card
}

fn preview_description(
    mtm: MainThreadMarker,
    text: &str,
    identifier: &str,
) -> Retained<NSTextField> {
    let description =
        NSTextField::labelWithString(&objc2_foundation::NSString::from_str(text), mtm);
    description.setFrame(NSRect::new(
        NSPoint::new(16.0, 10.0),
        NSSize::new(276.0, 36.0),
    ));
    description.setMaximumNumberOfLines(2);
    description.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    description.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(identifier)));
    description
}

fn create_footer(
    mtm: MainThreadMarker,
    contract: PreferencesShellContract,
    indicator_root: &NSView,
) -> PreferencesFooter {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        fixed_region_frame(
            contract,
            PreferencesRegion::Footer,
            NSRect::new(NSPoint::new(24.0, 20.0), NSSize::new(632.0, 56.0)),
        ),
    );
    let ids = contract.accessibility_identifiers();
    let reset_all = footer_placeholder(
        mtm,
        "Restaurar indicador aos padrões…",
        ids[3],
        NSRect::new(NSPoint::new(0.0, 10.0), NSSize::new(230.0, 34.0)),
    );
    let undo = footer_placeholder(
        mtm,
        "Desfazer restauração",
        ids[4],
        NSRect::new(NSPoint::new(244.0, 10.0), NSSize::new(170.0, 34.0)),
    );
    let retry_save = footer_placeholder(
        mtm,
        "Tentar novamente",
        ids[5],
        NSRect::new(NSPoint::new(448.0, 10.0), NSSize::new(164.0, 34.0)),
    );
    for button in [&reset_all, &undo, &retry_save] {
        button.setEnabled(false);
        button.setHidden(contract.footer_placeholders_are_hidden_and_disabled());
        view.addSubview(button);
    }
    indicator_root.addSubview(&view);
    PreferencesFooter {
        _view: view,
        _reset_all: reset_all,
        _undo: undo,
        _retry_save: retry_save,
    }
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

fn footer_placeholder(
    mtm: MainThreadMarker,
    title: &str,
    identifier: &str,
    frame: NSRect,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &objc2_foundation::NSString::from_str(title),
            None,
            None,
            mtm,
        )
    };
    button.setFrame(frame);
    button.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(identifier)));
    button
}

fn create_disk_and_mole_page(mtm: MainThreadMarker, target: &ControlTarget) -> DiskAndMolePage {
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(680.0, 640.0)),
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
    root.addSubview(&content);

    DiskAndMolePage {
        root,
        mole_checkbox: checkbox,
        warning_threshold: threshold,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::{
        get_or_create_window, PreferencesArea, PreferencesAreaState, PreferencesRegion,
        PreferencesShellContract, RegionPlacement,
    };

    #[test]
    fn selecting_an_area_shows_exactly_one_preferences_page() {
        let areas = [PreferencesArea::Indicator, PreferencesArea::DiskAndMole];
        let state = PreferencesAreaState::new();

        assert_eq!(state.visible(), PreferencesArea::Indicator);
        assert_eq!(
            areas.iter().filter(|area| state.is_visible(**area)).count(),
            1
        );

        let state = state.select(PreferencesArea::DiskAndMole);
        assert_eq!(state.visible(), PreferencesArea::DiskAndMole);
        assert_eq!(
            areas.iter().filter(|area| state.is_visible(**area)).count(),
            1
        );
    }

    #[test]
    fn preferences_shell_contract_has_exact_size_and_selector_labels() {
        let contract = PreferencesShellContract::new();

        assert_eq!(contract.content_size(), (680.0, 700.0));
        assert_eq!(contract.selector_labels(), ["Indicador", "Disco e Mole"]);
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
        assert!(!contract.preview_images_are_accessibility_elements());
        assert!(contract.footer_placeholders_are_hidden_and_disabled());
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
}
