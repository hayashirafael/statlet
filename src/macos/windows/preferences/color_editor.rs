use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColor, NSColorSpace, NSColorWell, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSEvent, NSStackView, NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize,
};
use statlet::core::{AppEvent, IndicatorPreferenceChange};
use statlet::indicator_preferences::{IndicatorAppearance, MetricKind, SrgbColor};
use statlet::preferences_view::{ColorEditorState, HexDraftError, HexEdit};
use tao::event_loop::EventLoopProxy;

use crate::macos::RuntimeEvent;

#[derive(Clone)]
struct ColorRow {
    well: Retained<NSColorWell>,
    hex: Retained<NSTextField>,
    error: Retained<NSTextField>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorBinding {
    MetricShared(MetricKind),
    MetricAppearance(MetricKind, IndicatorAppearance),
    LabelShared,
    LabelAppearance(IndicatorAppearance),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowKind {
    Shared,
    Light,
    Dark,
}

define_class!(
    #[unsafe(super = NSColorWell)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct KeyboardColorWell;

    unsafe impl NSObjectProtocol for KeyboardColorWell {}

    impl KeyboardColorWell {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let activates = event
                .charactersIgnoringModifiers()
                .is_some_and(|characters| matches!(characters.to_string().as_str(), " " | "\r"));
            if activates {
                self.activate(true);
            } else {
                unsafe {
                    let _: () = msg_send![super(self), keyDown: event];
                }
            }
        }
    }
);

struct ColorEditorTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    binding: ColorBinding,
    applying: Cell<bool>,
    state: RefCell<ColorEditorState>,
    shared: ColorRow,
    light: ColorRow,
    dark: ColorRow,
    shared_view: Retained<NSView>,
    light_view: Retained<NSView>,
    dark_view: Retained<NSView>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ColorEditorTargetIvars]
    struct ColorEditorTarget;

    unsafe impl NSObjectProtocol for ColorEditorTarget {}

    impl ColorEditorTarget {
        #[unsafe(method(changeColorWell:))]
        fn change_color_well(&self, sender: &NSColorWell) {
            if self.ivars().applying.get() {
                return;
            }
            let Some(row) = self.row_for_well(sender) else {
                return;
            };
            let Some(color) = native_color_to_srgb(&sender.color()) else {
                return;
            };
            self.set_draft_color(row, color);
            self.apply_row(row);
            self.send_color(row, color);
        }

        #[unsafe(method(commitHex:))]
        fn commit_hex(&self, sender: &NSTextField) {
            self.commit_field(sender);
        }

        #[unsafe(method(toggleAppearanceVariants:))]
        fn toggle_appearance_variants(&self, sender: &NSButton) {
            if self.ivars().applying.get() {
                return;
            }
            let enabled = sender.state() == NSControlStateValueOn;
            self.ivars()
                .state
                .borrow_mut()
                .set_variants_enabled(enabled);
            self.apply_visibility();
            let change = match self.ivars().binding {
                ColorBinding::MetricShared(metric)
                | ColorBinding::MetricAppearance(metric, _) => {
                    IndicatorPreferenceChange::SetMetricVariantsEnabled { metric, enabled }
                }
                ColorBinding::LabelShared | ColorBinding::LabelAppearance(_) => {
                    IndicatorPreferenceChange::SetLabelVariantsEnabled(enabled)
                }
            };
            self.send(change);
        }
    }

    unsafe impl NSControlTextEditingDelegate for ColorEditorTarget {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            if self.ivars().applying.get() {
                return;
            }
            let Some(field) = notification_text_field(notification) else {
                return;
            };
            self.edit_field(&field);
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, notification: &NSNotification) {
            if self.ivars().applying.get() {
                return;
            }
            let Some(field) = notification_text_field(notification) else {
                return;
            };
            self.commit_field(&field);
        }
    }

    unsafe impl NSTextFieldDelegate for ColorEditorTarget {}
);

impl ColorEditorTarget {
    fn new(mtm: MainThreadMarker, ivars: ColorEditorTargetIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    fn row_for_well(&self, well: &NSColorWell) -> Option<RowKind> {
        if std::ptr::eq(well, &*self.ivars().shared.well) {
            Some(RowKind::Shared)
        } else if std::ptr::eq(well, &*self.ivars().light.well) {
            Some(RowKind::Light)
        } else if std::ptr::eq(well, &*self.ivars().dark.well) {
            Some(RowKind::Dark)
        } else {
            None
        }
    }

    fn row_for_field(&self, field: &NSTextField) -> Option<RowKind> {
        if std::ptr::eq(field, &*self.ivars().shared.hex) {
            Some(RowKind::Shared)
        } else if std::ptr::eq(field, &*self.ivars().light.hex) {
            Some(RowKind::Light)
        } else if std::ptr::eq(field, &*self.ivars().dark.hex) {
            Some(RowKind::Dark)
        } else {
            None
        }
    }

    fn row(&self, row: RowKind) -> &ColorRow {
        match row {
            RowKind::Shared => &self.ivars().shared,
            RowKind::Light => &self.ivars().light,
            RowKind::Dark => &self.ivars().dark,
        }
    }

    fn edit_field(&self, field: &NSTextField) {
        let Some(row) = self.row_for_field(field) else {
            return;
        };
        let text = field.stringValue().to_string();
        let result = {
            let mut state = self.ivars().state.borrow_mut();
            draft_mut(&mut state, row).edit(&text)
        };
        set_error(&self.row(row).error, None);
        if let HexEdit::Applied(color) = result {
            self.apply_row(row);
            self.send_color(row, color);
        }
    }

    fn commit_field(&self, field: &NSTextField) {
        let Some(row) = self.row_for_field(field) else {
            return;
        };
        let (previous, result) = {
            let mut state = self.ivars().state.borrow_mut();
            let draft = draft_mut(&mut state, row);
            let previous = draft.valid_color();
            (previous, draft.commit())
        };
        match result {
            Ok(color) => {
                self.apply_row(row);
                if color != previous {
                    self.send_color(row, color);
                }
            }
            Err(error) => set_error(&self.row(row).error, Some(error)),
        }
    }

    fn set_draft_color(&self, row: RowKind, color: SrgbColor) {
        let mut state = self.ivars().state.borrow_mut();
        draft_mut(&mut state, row).set_color(color);
    }

    fn apply_row(&self, row: RowKind) {
        let draft = {
            let state = self.ivars().state.borrow();
            draft(&state, row).clone()
        };
        let controls = self.row(row);
        controls.well.setColor(&native_color(draft.valid_color()));
        controls
            .hex
            .setStringValue(&objc2_foundation::NSString::from_str(draft.text()));
        set_error(&controls.error, draft.error());
    }

    fn apply_visibility(&self) {
        let variants = self.ivars().state.borrow().variants_enabled();
        self.ivars().shared_view.setHidden(variants);
        self.ivars().light_view.setHidden(!variants);
        self.ivars().dark_view.setHidden(!variants);
    }

    fn send_color(&self, row: RowKind, color: SrgbColor) {
        let binding = match row {
            RowKind::Shared => self.ivars().binding,
            RowKind::Light => self
                .ivars()
                .binding
                .for_appearance(IndicatorAppearance::Light),
            RowKind::Dark => self
                .ivars()
                .binding
                .for_appearance(IndicatorAppearance::Dark),
        };
        let change = match binding {
            ColorBinding::MetricShared(metric) => {
                IndicatorPreferenceChange::SetMetricSharedColor { metric, color }
            }
            ColorBinding::MetricAppearance(metric, appearance) => {
                IndicatorPreferenceChange::SetMetricAppearanceColor {
                    metric,
                    appearance,
                    color,
                }
            }
            ColorBinding::LabelShared => IndicatorPreferenceChange::SetLabelSharedColor(color),
            ColorBinding::LabelAppearance(appearance) => {
                IndicatorPreferenceChange::SetLabelAppearanceColor { appearance, color }
            }
        };
        self.send(change);
    }

    fn send(&self, change: IndicatorPreferenceChange) {
        let _ = self
            .ivars()
            .proxy
            .send_event(RuntimeEvent::App(AppEvent::UpdateIndicator(change)));
    }
}

pub struct ColorEditor {
    view: Retained<NSStackView>,
    shared: ColorRow,
    light: ColorRow,
    dark: ColorRow,
    variants_toggle: Retained<NSButton>,
    target: Retained<ColorEditorTarget>,
}

impl ColorEditor {
    pub fn new(
        mtm: MainThreadMarker,
        binding: ColorBinding,
        proxy: EventLoopProxy<RuntimeEvent>,
    ) -> Self {
        let prefix = accessibility_prefix(binding);
        let view = NSStackView::initWithFrame(
            NSStackView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(540.0, 160.0)),
        );
        let (shared, shared_view) =
            create_color_row(mtm, "Cor", prefix, shared_hex_identifier(binding), 106.0);
        let light_id = format!("{}.light", prefix.trim_end_matches(".color"));
        let dark_id = format!("{}.dark", prefix.trim_end_matches(".color"));
        let light_hex_id = format!("{light_id}.hex");
        let dark_hex_id = format!("{dark_id}.hex");
        let (light, light_view) = create_color_row(mtm, "Claro", &light_id, &light_hex_id, 106.0);
        let (dark, dark_view) = create_color_row(mtm, "Escuro", &dark_id, &dark_hex_id, 58.0);
        for row_view in [&shared_view, &light_view, &dark_view] {
            view.addSubview(row_view);
        }

        let variants_toggle = unsafe {
            NSButton::checkboxWithTitle_target_action(
                ns_string!("Personalizar claro e escuro"),
                None,
                None,
                mtm,
            )
        };
        variants_toggle.setFrame(NSRect::new(
            NSPoint::new(100.0, 14.0),
            NSSize::new(260.0, 24.0),
        ));
        variants_toggle.setAccessibilityLabel(Some(ns_string!("Personalizar claro e escuro")));
        variants_toggle.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(
            &format!("{prefix}.variants"),
        )));
        view.addSubview(&variants_toggle);

        let initial = statlet::indicator_preferences::IndicatorPreferences::default();
        let preferences = match binding {
            ColorBinding::MetricShared(MetricKind::Cpu)
            | ColorBinding::MetricAppearance(MetricKind::Cpu, _) => initial.cpu_color.fixed,
            ColorBinding::MetricShared(MetricKind::Ram)
            | ColorBinding::MetricAppearance(MetricKind::Ram, _) => initial.ram_color.fixed,
            ColorBinding::LabelShared | ColorBinding::LabelAppearance(_) => initial.labels.fixed,
        };
        let target = ColorEditorTarget::new(
            mtm,
            ColorEditorTargetIvars {
                proxy,
                binding,
                applying: Cell::new(false),
                state: RefCell::new(ColorEditorState::from_preferences(preferences)),
                shared: shared.clone(),
                light: light.clone(),
                dark: dark.clone(),
                shared_view,
                light_view,
                dark_view,
            },
        );

        for row in [&shared, &light, &dark] {
            unsafe {
                row.well.setTarget(Some(&*target as &AnyObject));
                row.well.setAction(Some(sel!(changeColorWell:)));
                row.hex.setTarget(Some(&*target as &AnyObject));
                row.hex.setAction(Some(sel!(commitHex:)));
                row.hex
                    .setDelegate(Some(ProtocolObject::from_ref(&*target)));
            }
        }
        unsafe {
            variants_toggle.setTarget(Some(&*target as &AnyObject));
            variants_toggle.setAction(Some(sel!(toggleAppearanceVariants:)));
        }

        let editor = Self {
            view,
            shared,
            light,
            dark,
            variants_toggle,
            target,
        };
        let state = ColorEditorState::from_preferences(preferences);
        editor.apply(&state);
        editor
    }

    pub fn apply(&self, state: &ColorEditorState) {
        self.target.ivars().applying.set(true);
        self.target.ivars().state.borrow_mut().sync_from(state);
        self.variants_toggle.setState(if state.variants_enabled() {
            NSControlStateValueOn
        } else {
            0
        });
        self.target.apply_visibility();
        for row in [RowKind::Shared, RowKind::Light, RowKind::Dark] {
            self.target.apply_row(row);
        }
        self.target.ivars().applying.set(false);
    }

    pub fn deactivate(&self) {
        self.shared.well.deactivate();
        self.light.well.deactivate();
        self.dark.well.deactivate();
    }

    pub(super) fn view(&self) -> &NSStackView {
        &self.view
    }

    pub(super) fn wells(&self) -> [Retained<NSColorWell>; 3] {
        [
            self.shared.well.clone(),
            self.light.well.clone(),
            self.dark.well.clone(),
        ]
    }

    pub(super) fn configure_key_order(
        &self,
        previous: &NSView,
        next: &NSView,
        state: &ColorEditorState,
    ) {
        unsafe {
            if state.variants_enabled() {
                previous.setNextKeyView(Some(&self.light.well));
                self.light.well.setNextKeyView(Some(&self.light.hex));
                self.light.hex.setNextKeyView(Some(&self.dark.well));
                self.dark.well.setNextKeyView(Some(&self.dark.hex));
                self.dark.hex.setNextKeyView(Some(&self.variants_toggle));
            } else {
                previous.setNextKeyView(Some(&self.shared.well));
                self.shared.well.setNextKeyView(Some(&self.shared.hex));
                self.shared.hex.setNextKeyView(Some(&self.variants_toggle));
            }
            self.variants_toggle.setNextKeyView(Some(next));
        }
    }
}

fn create_color_row(
    mtm: MainThreadMarker,
    title: &str,
    identifier_prefix: &str,
    hex_identifier: &str,
    y: f64,
) -> (ColorRow, Retained<NSView>) {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, y), NSSize::new(540.0, 46.0)),
    );
    let label = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(title), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(0.0, 18.0),
        NSSize::new(92.0, 22.0),
    ));
    view.addSubview(&label);

    let well: Retained<KeyboardColorWell> = unsafe {
        msg_send![
            KeyboardColorWell::alloc(mtm),
            initWithFrame: NSRect::new(NSPoint::new(100.0, 12.0), NSSize::new(44.0, 30.0))
        ]
    };
    let well: Retained<NSColorWell> = Retained::into_super(well);
    well.setSupportsAlpha(false);
    well.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Cor {title}"
    ))));
    well.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{identifier_prefix}.well"
    ))));
    view.addSubview(&well);

    let hex = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(156.0, 12.0), NSSize::new(112.0, 28.0)),
    );
    hex.setPlaceholderString(Some(ns_string!("#RRGGBB")));
    hex.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Cor {title} em hexadecimal"
    ))));
    hex.setAccessibilityHelp(Some(ns_string!(
        "Digite seis dígitos hexadecimais e pressione Return."
    )));
    hex.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(hex_identifier)));
    view.addSubview(&hex);

    let error = NSTextField::labelWithString(ns_string!(""), mtm);
    error.setFrame(NSRect::new(
        NSPoint::new(280.0, 8.0),
        NSSize::new(250.0, 34.0),
    ));
    error.setMaximumNumberOfLines(2);
    error.setTextColor(Some(&NSColor::systemRedColor()));
    error.setAccessibilityIdentifier(Some(&objc2_foundation::NSString::from_str(&format!(
        "{identifier_prefix}.error"
    ))));
    error.setHidden(true);
    view.addSubview(&error);

    (ColorRow { well, hex, error }, view)
}

fn draft(state: &ColorEditorState, row: RowKind) -> &statlet::preferences_view::HexDraft {
    match row {
        RowKind::Shared => state.shared(),
        RowKind::Light => state.light(),
        RowKind::Dark => state.dark(),
    }
}

fn draft_mut(
    state: &mut ColorEditorState,
    row: RowKind,
) -> &mut statlet::preferences_view::HexDraft {
    match row {
        RowKind::Shared => state.shared_mut(),
        RowKind::Light => state.light_mut(),
        RowKind::Dark => state.dark_mut(),
    }
}

fn notification_text_field(notification: &NSNotification) -> Option<Retained<NSTextField>> {
    notification.object()?.downcast::<NSTextField>().ok()
}

fn set_error(field: &NSTextField, error: Option<HexDraftError>) {
    let message = error.map(HexDraftError::message).unwrap_or("");
    let message = objc2_foundation::NSString::from_str(message);
    field.setStringValue(&message);
    field.setAccessibilityLabel(error.map(|_| &*message));
    field.setHidden(error.is_none());
}

fn native_color(color: SrgbColor) -> Retained<NSColor> {
    let [red, green, blue] = color
        .components()
        .map(|component| f64::from(component) / 255.0);
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0)
}

fn native_color_to_srgb(color: &NSColor) -> Option<SrgbColor> {
    let color = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let component = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let hex = format!(
        "#{:02X}{:02X}{:02X}",
        component(color.redComponent()),
        component(color.greenComponent()),
        component(color.blueComponent())
    );
    SrgbColor::parse_hex(&hex).ok()
}

fn accessibility_prefix(binding: ColorBinding) -> &'static str {
    match binding {
        ColorBinding::MetricShared(MetricKind::Cpu)
        | ColorBinding::MetricAppearance(MetricKind::Cpu, _) => "indicator.cpu.color",
        ColorBinding::MetricShared(MetricKind::Ram)
        | ColorBinding::MetricAppearance(MetricKind::Ram, _) => "indicator.ram.color",
        ColorBinding::LabelShared | ColorBinding::LabelAppearance(_) => "indicator.labels.color",
    }
}

fn shared_hex_identifier(binding: ColorBinding) -> &'static str {
    match binding {
        ColorBinding::MetricShared(MetricKind::Cpu)
        | ColorBinding::MetricAppearance(MetricKind::Cpu, _) => "indicator.cpu.color.hex",
        ColorBinding::MetricShared(MetricKind::Ram)
        | ColorBinding::MetricAppearance(MetricKind::Ram, _) => "indicator.ram.color.hex",
        ColorBinding::LabelShared | ColorBinding::LabelAppearance(_) => {
            "indicator.labels.color.hex"
        }
    }
}

impl ColorBinding {
    const fn for_appearance(self, appearance: IndicatorAppearance) -> Self {
        match self {
            Self::MetricShared(metric) | Self::MetricAppearance(metric, _) => {
                Self::MetricAppearance(metric, appearance)
            }
            Self::LabelShared | Self::LabelAppearance(_) => Self::LabelAppearance(appearance),
        }
    }
}
