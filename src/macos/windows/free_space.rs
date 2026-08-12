use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::sel;
use objc2_app_kit::{NSAccessibility, NSButton, NSLineBreakMode, NSTextField, NSWindow};
use objc2_foundation::{ns_string, MainThreadMarker, NSFileManager, NSPoint, NSRect, NSSize};
use statlet::core::AppState;
use statlet::disk::format_decimal_gigabytes;
use statlet::mole::MoleStatus;

use super::common::{create_window, text_label, ControlTarget};

pub(super) struct FreeSpaceWindow {
    pub(super) window: Retained<NSWindow>,
    occupied_value: Retained<NSTextField>,
    available_value: Retained<NSTextField>,
    threshold_value: Retained<NSTextField>,
    mole_status: Retained<NSTextField>,
    open_mole_button: Retained<NSButton>,
}

impl FreeSpaceWindow {
    pub(super) fn apply(&self, state: &AppState) {
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

pub(super) fn create_free_space_window(
    mtm: MainThreadMarker,
    target: &ControlTarget,
) -> FreeSpaceWindow {
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
