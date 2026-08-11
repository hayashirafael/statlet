use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSBackingStoreType, NSButton,
    NSControlStateValueOn, NSLineBreakMode, NSPopUpButton, NSScrollView, NSTextField, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSDate, NSDateFormatter, NSDateFormatterStyle, NSFileManager,
    NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};
use statlet::core::{AppEvent, AppState, Preferences, WarningThreshold, WindowKind};
use statlet::disk::format_decimal_gigabytes;
use statlet::history::{History, HistoryEventKind, HistoryRecord, MAX_HISTORY_RECORDS};
use statlet::mole::MoleStatus;
use tao::event_loop::EventLoopProxy;

use super::RuntimeEvent;

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

impl WindowManager {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let control_target = ControlTarget::new(mtm, proxy);
        Self {
            control_target,
            preferences: None,
            history: None,
            free_space: None,
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
        };

        window.makeKeyAndOrderFront(None);
        NSApplication::sharedApplication(mtm).activate();
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
        self.available_value
            .setStringValue(&objc2_foundation::NSString::from_str(&available));
        self.threshold_value
            .setStringValue(&objc2_foundation::NSString::from_str(&format!(
                "{}%",
                state.preferences.warning_threshold.get()
            )));

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

    content.addSubview(&heading);
    content.addSubview(&checkbox);
    content.addSubview(&explanation);
    content.addSubview(&threshold_label);
    content.addSubview(&threshold);

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

    content.addSubview(&heading);
    content.addSubview(&explanation);
    content.addSubview(&empty_label);
    content.addSubview(&scroll_view);
    content.addSubview(&clear_button);

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
    window.setTitle(&objc2_foundation::NSString::from_str(title));
    window.center();
    window
}

fn threshold_title(threshold: WarningThreshold) -> Retained<objc2_foundation::NSString> {
    objc2_foundation::NSString::from_str(&format!("{}%", threshold.get()))
}
