use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{sel, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSLineBreakMode, NSScrollView, NSTextField, NSView, NSWindow,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSDate, NSDateFormatter, NSDateFormatterStyle, NSPoint, NSRect,
    NSSize,
};
use statlet::history::{History, HistoryEventKind, HistoryRecord, MAX_HISTORY_RECORDS};

use super::common::{create_window, text_label, ControlTarget};

pub(super) struct HistoryWindow {
    pub(super) window: Retained<NSWindow>,
    document: Retained<NSView>,
    rows: Vec<Retained<NSTextField>>,
    empty_label: Retained<NSTextField>,
    scroll_view: Retained<NSScrollView>,
    clear_button: Retained<NSButton>,
}

impl HistoryWindow {
    pub(super) fn apply(&self, history: &History) {
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

pub(super) fn create_history_window(
    mtm: MainThreadMarker,
    target: &ControlTarget,
) -> HistoryWindow {
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
