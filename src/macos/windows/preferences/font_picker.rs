use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSControlTextEditingDelegate, NSFont, NSFontManager, NSFontTraitMask,
    NSScrollView, NSSearchField, NSTableColumn, NSTableView, NSTableViewDataSource,
    NSTableViewDelegate, NSTextField, NSUserInterfaceItemIdentifier, NSView, NSWindow,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize,
};
use statlet::core::{AppEvent, IndicatorPreferenceChange};
use statlet::indicator_preferences::FontFamilyPreference;
use statlet::preferences_view::{filter_font_families, FontRow};
use tao::event_loop::EventLoopProxy;

use super::super::common::create_window;
use crate::macos::fonts::FontCatalog;
use crate::macos::RuntimeEvent;

struct FontPickerDataSourceIvars {
    rows: Rc<RefCell<Vec<FontRow>>>,
    families: RefCell<Vec<String>>,
    missing_selection: RefCell<Option<String>>,
    table: Retained<NSTableView>,
    suppress_selection: Rc<Cell<bool>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = FontPickerDataSourceIvars]
    struct FontPickerDataSource;

    unsafe impl NSObjectProtocol for FontPickerDataSource {}

    impl FontPickerDataSource {
        #[unsafe(method(filterFonts:))]
        fn filter_fonts(&self, sender: &NSSearchField) {
            self.refilter(&sender.stringValue().to_string());
        }
    }

    unsafe impl NSTableViewDataSource for FontPickerDataSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows_in_table_view(&self, _table: &NSTableView) -> NSInteger {
            self.ivars().rows.borrow().len() as NSInteger
        }
    }
);

impl FontPickerDataSource {
    fn new(
        mtm: MainThreadMarker,
        rows: Rc<RefCell<Vec<FontRow>>>,
        table: Retained<NSTableView>,
        suppress_selection: Rc<Cell<bool>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FontPickerDataSourceIvars {
            rows,
            families: RefCell::new(Vec::new()),
            missing_selection: RefCell::new(None),
            table,
            suppress_selection,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn update(&self, families: &[String], missing_selection: Option<&str>, query: &str) {
        self.ivars().families.replace(families.to_vec());
        self.ivars()
            .missing_selection
            .replace(missing_selection.map(str::to_owned));
        self.refilter(query);
    }

    fn refresh_families(&self, families: &[String], query: &str) {
        self.ivars().families.replace(families.to_vec());
        self.refilter(query);
    }

    fn refilter(&self, query: &str) {
        let families = self.ivars().families.borrow();
        let missing = self.ivars().missing_selection.borrow();
        self.ivars()
            .rows
            .replace(filter_font_families(&families, query, missing.as_deref()));
        self.ivars().suppress_selection.set(true);
        self.ivars().table.reloadData();
        unsafe {
            self.ivars().table.deselectAll(None);
        }
        self.ivars().suppress_selection.set(false);
    }
}

struct FontPickerDelegateIvars {
    rows: Rc<RefCell<Vec<FontRow>>>,
    table: Retained<NSTableView>,
    sheet: Retained<NSWindow>,
    proxy: RefCell<EventLoopProxy<RuntimeEvent>>,
    suppress_selection: Rc<Cell<bool>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = FontPickerDelegateIvars]
    struct FontPickerDelegate;

    unsafe impl NSObjectProtocol for FontPickerDelegate {}
    unsafe impl NSControlTextEditingDelegate for FontPickerDelegate {}

    unsafe impl NSTableViewDelegate for FontPickerDelegate {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn table_view_view_for_table_column_row(
            &self,
            _table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            let row = self.ivars().rows.borrow()[row as usize].clone();
            Some(font_row_view(row).into_super().into_super())
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn table_view_selection_did_change(&self, _notification: &NSNotification) {
            if self.ivars().suppress_selection.get() {
                return;
            }
            let Ok(index) = usize::try_from(self.ivars().table.selectedRow()) else {
                return;
            };
            let Some(row) = self.ivars().rows.borrow().get(index).cloned() else {
                return;
            };
            let _ = self.ivars().proxy.borrow().send_event(RuntimeEvent::App(
                AppEvent::UpdateIndicator(IndicatorPreferenceChange::SetFontFamily(
                    row.family_preference(),
                )),
            ));
            if let Some(parent) = self.ivars().sheet.sheetParent() {
                parent.endSheet(&self.ivars().sheet);
            }
        }
    }
);

impl FontPickerDelegate {
    fn new(
        mtm: MainThreadMarker,
        rows: Rc<RefCell<Vec<FontRow>>>,
        table: Retained<NSTableView>,
        sheet: Retained<NSWindow>,
        proxy: EventLoopProxy<RuntimeEvent>,
        suppress_selection: Rc<Cell<bool>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FontPickerDelegateIvars {
            rows,
            table,
            sheet,
            proxy: RefCell::new(proxy),
            suppress_selection,
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub struct FontPicker {
    sheet: Retained<NSWindow>,
    search: Retained<NSSearchField>,
    table: Retained<NSTableView>,
    data_source: Retained<FontPickerDataSource>,
    delegate: Retained<FontPickerDelegate>,
    proxy: EventLoopProxy<RuntimeEvent>,
}

impl FontPicker {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let sheet = create_window(mtm, "Escolher fonte", NSSize::new(520.0, 430.0));
        let content = sheet.contentView().expect("font picker content view");

        let search = NSSearchField::initWithFrame(
            NSSearchField::alloc(mtm),
            NSRect::new(NSPoint::new(20.0, 382.0), NSSize::new(480.0, 28.0)),
        );
        search.setPlaceholderString(Some(ns_string!("Buscar fontes")));
        search.setSendsSearchStringImmediately(true);
        search.setAccessibilityLabel(Some(ns_string!("Buscar fontes")));
        search.setAccessibilityIdentifier(Some(ns_string!("indicator.font.search")));
        content.addSubview(&search);

        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(NSPoint::new(20.0, 20.0), NSSize::new(480.0, 350.0)),
        );
        scroll.setHasVerticalScroller(true);
        let table = NSTableView::initWithFrame(
            NSTableView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 350.0)),
        );
        table.setAllowsMultipleSelection(false);
        table.setAllowsEmptySelection(true);
        table.setRowHeight(42.0);
        table.setAccessibilityLabel(Some(ns_string!("Famílias de fontes")));
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            &NSUserInterfaceItemIdentifier::from_str("family"),
        );
        column.setWidth(476.0);
        table.addTableColumn(&column);
        scroll.setDocumentView(Some(&table));
        content.addSubview(&scroll);

        let rows = Rc::new(RefCell::new(Vec::new()));
        let suppress_selection = Rc::new(Cell::new(false));
        let data_source =
            FontPickerDataSource::new(mtm, rows.clone(), table.clone(), suppress_selection.clone());
        let delegate = FontPickerDelegate::new(
            mtm,
            rows,
            table.clone(),
            sheet.clone(),
            proxy.clone(),
            suppress_selection,
        );
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(&*data_source)));
            table.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            search.setTarget(Some(&*data_source as &AnyObject));
            search.setAction(Some(sel!(filterFonts:)));
            search.setNextKeyView(Some(&table));
            table.setNextKeyView(Some(&search));
        }

        Self {
            sheet,
            search,
            table,
            data_source,
            delegate,
            proxy,
        }
    }

    pub fn present(
        &mut self,
        parent: &NSWindow,
        catalog: &FontCatalog,
        selected: &FontFamilyPreference,
    ) {
        self.delegate.ivars().proxy.replace(self.proxy.clone());
        self.search.setStringValue(ns_string!(""));
        let missing = missing_family(catalog, selected);
        self.data_source
            .update(catalog.families(), missing.as_deref(), "");

        self.delegate.ivars().suppress_selection.set(true);
        if let Some(index) = selected_row(&self.delegate.ivars().rows.borrow(), selected) {
            self.table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
            self.table.scrollRowToVisible(index as NSInteger);
        }
        self.delegate.ivars().suppress_selection.set(false);

        parent.beginSheet_completionHandler(&self.sheet, None);
        self.sheet.makeFirstResponder(Some(&self.search));
    }

    pub fn refresh_catalog(&mut self, catalog: &FontCatalog) {
        self.data_source
            .refresh_families(catalog.families(), &self.search.stringValue().to_string());
    }
}

fn font_row_view(row: FontRow) -> Retained<NSTextField> {
    let mtm = MainThreadMarker::new().expect("font picker rows run on the main thread");
    let suffix = if row.is_missing() {
        " — não disponível (usando System Monospaced)"
    } else {
        ""
    };
    let text = format!("{}{suffix}\nC 42% / R 68%", row.name());
    let field = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(&text), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(8.0, 2.0),
        NSSize::new(456.0, 38.0),
    ));
    field.setMaximumNumberOfLines(2);
    field.setFont(Some(&sample_font(mtm, &row)));
    field.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&format!(
        "Fonte {}. Amostra C 42 por cento, R 68 por cento.{suffix}",
        row.name()
    ))));
    field
}

fn sample_font(mtm: MainThreadMarker, row: &FontRow) -> Retained<NSFont> {
    match row {
        FontRow::Available(family) => NSFontManager::sharedFontManager(mtm)
            .fontWithFamily_traits_weight_size(
                &objc2_foundation::NSString::from_str(family),
                NSFontTraitMask::empty(),
                6,
                13.0,
            )
            .unwrap_or_else(|| NSFont::monospacedSystemFontOfSize_weight(13.0, medium_weight())),
        FontRow::SystemMonospaced | FontRow::Missing(_) => {
            NSFont::monospacedSystemFontOfSize_weight(13.0, medium_weight())
        }
    }
}

fn medium_weight() -> f64 {
    unsafe { objc2_app_kit::NSFontWeightMedium }
}

fn missing_family(catalog: &FontCatalog, selected: &FontFamilyPreference) -> Option<String> {
    let FontFamilyPreference::Named(selected) = selected else {
        return None;
    };
    (!catalog
        .families()
        .iter()
        .any(|family| family.to_lowercase() == selected.to_lowercase()))
    .then(|| selected.clone())
}

fn selected_row(rows: &[FontRow], selected: &FontFamilyPreference) -> Option<usize> {
    rows.iter().position(|row| match (row, selected) {
        (FontRow::SystemMonospaced, FontFamilyPreference::SystemMonospaced) => true,
        (
            FontRow::Available(row) | FontRow::Missing(row),
            FontFamilyPreference::Named(selected),
        ) => row.to_lowercase() == selected.to_lowercase(),
        _ => false,
    })
}
