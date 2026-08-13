pub const INLINE_GAP: f64 = 12.0;
pub const GROUP_GAP: f64 = 24.0;
pub const COLOR_EDITOR_HEIGHT: f64 = 160.0;

const HEADING_HEIGHT: f64 = 24.0;
const ROW_HEIGHT: f64 = 28.0;
const MESSAGE_HEIGHT: f64 = 20.0;
const CONTROL_X: f64 = 100.0;

pub fn preserve_scroll_origin_from_top(
    origin_y: f64,
    viewport_height: f64,
    old_document_height: f64,
    new_document_height: f64,
) -> f64 {
    let old_max_origin = (old_document_height - viewport_height).max(0.0);
    let new_max_origin = (new_document_height - viewport_height).max(0.0);
    let top_offset = old_max_origin - origin_y.clamp(0.0, old_max_origin);

    (new_max_origin - top_offset).clamp(0.0, new_max_origin)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IndicatorControlsVisibility {
    pub cpu_editor: bool,
    pub ram_editor: bool,
    pub labels_editor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalSlot {
    top: f64,
    height: f64,
}

impl VerticalSlot {
    pub const fn top(self) -> f64 {
        self.top
    }

    pub const fn height(self) -> f64 {
        self.height
    }

    pub const fn bottom(self) -> f64 {
        self.top + self.height
    }

    pub const fn origin_y(self, content_height: f64) -> f64 {
        content_height - self.bottom()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowSlot {
    vertical: VerticalSlot,
    label_x: f64,
    control_x: f64,
}

impl RowSlot {
    pub const fn top(self) -> f64 {
        self.vertical.top()
    }

    pub const fn height(self) -> f64 {
        self.vertical.height()
    }

    pub const fn bottom(self) -> f64 {
        self.vertical.bottom()
    }

    pub const fn vertical(self) -> VerticalSlot {
        self.vertical
    }

    pub const fn label_x(self) -> f64 {
        self.label_x
    }

    pub const fn control_x(self) -> f64 {
        self.control_x
    }

    pub const fn origin_y(self, content_height: f64) -> f64 {
        self.vertical.origin_y(content_height)
    }

    pub const fn label_origin_y(self, content_height: f64) -> f64 {
        self.origin_y(content_height)
    }

    pub const fn control_origin_y(self, content_height: f64) -> f64 {
        self.origin_y(content_height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlSlot {
    vertical: VerticalSlot,
    x: f64,
    width: f64,
}

impl ControlSlot {
    pub const fn top(self) -> f64 {
        self.vertical.top()
    }

    pub const fn height(self) -> f64 {
        self.vertical.height()
    }

    pub const fn bottom(self) -> f64 {
        self.vertical.bottom()
    }

    pub const fn vertical(self) -> VerticalSlot {
        self.vertical
    }

    pub const fn x(self) -> f64 {
        self.x
    }

    pub const fn width(self) -> f64 {
        self.width
    }

    pub const fn origin_y(self, content_height: f64) -> f64 {
        self.vertical.origin_y(content_height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndicatorControlsLayout {
    colors_heading: VerticalSlot,
    colors_reset: ControlSlot,
    cpu_row: RowSlot,
    cpu_editor: Option<VerticalSlot>,
    ram_row: RowSlot,
    ram_editor: Option<VerticalSlot>,
    labels_heading: VerticalSlot,
    labels_visibility_row: RowSlot,
    labels_mode_row: RowSlot,
    labels_editor: Option<VerticalSlot>,
    typography_heading: VerticalSlot,
    family_row: RowSlot,
    size_row: RowSlot,
    weight_row: RowSlot,
    font_fallback_warning: VerticalSlot,
    layout_warning: VerticalSlot,
    update_heading: VerticalSlot,
    interval_row: RowSlot,
    interval_help: VerticalSlot,
    interval_error: VerticalSlot,
    content_height: f64,
}

impl IndicatorControlsLayout {
    pub fn new(visibility: IndicatorControlsVisibility) -> Self {
        let mut cursor = 0.0;
        let colors_heading = vertical(&mut cursor, HEADING_HEIGHT);
        let colors_reset = control(colors_heading, 390.0, 160.0);

        cursor += INLINE_GAP;
        let cpu_row = row(&mut cursor);
        let cpu_editor = optional_editor(&mut cursor, visibility.cpu_editor);
        cursor += INLINE_GAP;
        let ram_row = row(&mut cursor);
        let ram_editor = optional_editor(&mut cursor, visibility.ram_editor);

        cursor += GROUP_GAP;
        let labels_heading = vertical(&mut cursor, HEADING_HEIGHT);
        cursor += INLINE_GAP;
        let labels_visibility_row = row(&mut cursor);
        cursor += INLINE_GAP;
        let labels_mode_row = row(&mut cursor);
        let labels_editor = optional_editor(&mut cursor, visibility.labels_editor);

        cursor += GROUP_GAP;
        let typography_heading = vertical(&mut cursor, HEADING_HEIGHT);
        cursor += INLINE_GAP;
        let family_row = row(&mut cursor);
        cursor += INLINE_GAP;
        let size_row = row(&mut cursor);
        cursor += INLINE_GAP;
        let weight_row = row(&mut cursor);
        cursor += INLINE_GAP;
        let font_fallback_warning = vertical(&mut cursor, MESSAGE_HEIGHT);
        cursor += INLINE_GAP;
        let layout_warning = vertical(&mut cursor, MESSAGE_HEIGHT);

        cursor += GROUP_GAP;
        let update_heading = vertical(&mut cursor, HEADING_HEIGHT);
        cursor += INLINE_GAP;
        let interval_row = row(&mut cursor);
        cursor += INLINE_GAP;
        let interval_help = vertical(&mut cursor, MESSAGE_HEIGHT);
        cursor += INLINE_GAP;
        let interval_error = vertical(&mut cursor, MESSAGE_HEIGHT);

        Self {
            colors_heading,
            colors_reset,
            cpu_row,
            cpu_editor,
            ram_row,
            ram_editor,
            labels_heading,
            labels_visibility_row,
            labels_mode_row,
            labels_editor,
            typography_heading,
            family_row,
            size_row,
            weight_row,
            font_fallback_warning,
            layout_warning,
            update_heading,
            interval_row,
            interval_help,
            interval_error,
            content_height: cursor,
        }
    }

    pub const fn colors_heading(self) -> VerticalSlot {
        self.colors_heading
    }
    pub const fn colors_reset(self) -> ControlSlot {
        self.colors_reset
    }
    pub const fn cpu_row(self) -> RowSlot {
        self.cpu_row
    }
    pub const fn cpu_editor(self) -> Option<VerticalSlot> {
        self.cpu_editor
    }
    pub const fn ram_row(self) -> RowSlot {
        self.ram_row
    }
    pub const fn ram_editor(self) -> Option<VerticalSlot> {
        self.ram_editor
    }
    pub const fn labels_heading(self) -> VerticalSlot {
        self.labels_heading
    }
    pub const fn labels_visibility_row(self) -> RowSlot {
        self.labels_visibility_row
    }
    pub const fn labels_mode_row(self) -> RowSlot {
        self.labels_mode_row
    }
    pub const fn labels_editor(self) -> Option<VerticalSlot> {
        self.labels_editor
    }
    pub const fn typography_heading(self) -> VerticalSlot {
        self.typography_heading
    }
    pub const fn family_row(self) -> RowSlot {
        self.family_row
    }
    pub const fn size_row(self) -> RowSlot {
        self.size_row
    }
    pub const fn weight_row(self) -> RowSlot {
        self.weight_row
    }
    pub const fn font_fallback_warning(self) -> VerticalSlot {
        self.font_fallback_warning
    }
    pub const fn layout_warning(self) -> VerticalSlot {
        self.layout_warning
    }
    pub const fn update_heading(self) -> VerticalSlot {
        self.update_heading
    }
    pub const fn interval_row(self) -> RowSlot {
        self.interval_row
    }
    pub const fn interval_help(self) -> VerticalSlot {
        self.interval_help
    }
    pub const fn interval_error(self) -> VerticalSlot {
        self.interval_error
    }
    pub const fn content_height(self) -> f64 {
        self.content_height
    }
}

fn vertical(cursor: &mut f64, height: f64) -> VerticalSlot {
    let slot = VerticalSlot {
        top: *cursor,
        height,
    };
    *cursor += height;
    slot
}

fn row(cursor: &mut f64) -> RowSlot {
    RowSlot {
        vertical: vertical(cursor, ROW_HEIGHT),
        label_x: 0.0,
        control_x: CONTROL_X,
    }
}

fn control(vertical: VerticalSlot, x: f64, width: f64) -> ControlSlot {
    ControlSlot { vertical, x, width }
}

fn optional_editor(cursor: &mut f64, visible: bool) -> Option<VerticalSlot> {
    if visible {
        *cursor += INLINE_GAP;
        Some(vertical(cursor, COLOR_EDITOR_HEIGHT))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_colors_produce_the_smallest_layout_without_editor_slots() {
        let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

        assert_eq!(layout.cpu_editor(), None);
        assert_eq!(layout.ram_editor(), None);
        assert_eq!(layout.labels_editor(), None);
        assert!(layout.cpu_row().bottom() <= layout.ram_row().top());
        assert!(layout.ram_row().bottom() <= layout.labels_heading().top());
    }

    #[test]
    fn each_visible_editor_only_pushes_the_content_after_its_metric() {
        let compact = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());
        let cpu_fixed = IndicatorControlsLayout::new(IndicatorControlsVisibility {
            cpu_editor: true,
            ..IndicatorControlsVisibility::default()
        });

        assert_eq!(cpu_fixed.cpu_row(), compact.cpu_row());
        assert_eq!(
            cpu_fixed.cpu_editor().unwrap().height(),
            COLOR_EDITOR_HEIGHT
        );
        assert_eq!(
            cpu_fixed.ram_row().top() - compact.ram_row().top(),
            COLOR_EDITOR_HEIGHT + INLINE_GAP
        );
    }

    #[test]
    fn cpu_and_ram_rows_share_columns_and_the_reset_belongs_to_colors() {
        let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

        assert_eq!(layout.cpu_row().label_x(), layout.ram_row().label_x());
        assert_eq!(layout.cpu_row().control_x(), layout.ram_row().control_x());
        assert_eq!(layout.colors_reset().vertical(), layout.colors_heading());
        assert!(layout.colors_reset().x() > layout.cpu_row().control_x());
        assert_eq!(layout.colors_reset().width(), 160.0);
        assert_eq!(GROUP_GAP, 24.0);
    }

    #[test]
    fn slot_heights_leave_the_minimum_inline_gap_between_real_frames() {
        let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

        assert_eq!(layout.family_row().height(), ROW_HEIGHT);
        assert_eq!(layout.interval_help().height(), MESSAGE_HEIGHT);
        assert_eq!(
            layout.size_row().top() - layout.family_row().bottom(),
            INLINE_GAP
        );
        assert_eq!(
            layout.interval_help().top() - layout.interval_row().bottom(),
            INLINE_GAP
        );
    }

    #[test]
    fn top_down_slots_translate_to_appkit_without_changing_row_alignment() {
        let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());
        let cpu = layout.cpu_row();
        let ram = layout.ram_row();

        assert_eq!(
            cpu.origin_y(layout.content_height()),
            layout.content_height() - cpu.bottom()
        );
        assert_eq!(
            cpu.label_origin_y(layout.content_height()),
            cpu.control_origin_y(layout.content_height())
        );
        assert_eq!(
            ram.label_origin_y(layout.content_height()),
            ram.control_origin_y(layout.content_height())
        );
    }

    #[test]
    fn scroll_origin_preserves_top_offset_and_clamps_to_the_new_range() {
        assert_eq!(
            preserve_scroll_origin_from_top(304.0, 344.0, 648.0, 820.0),
            476.0
        );
        assert_eq!(
            preserve_scroll_origin_from_top(180.0, 344.0, 648.0, 820.0),
            352.0
        );
        assert_eq!(
            preserve_scroll_origin_from_top(180.0, 344.0, 648.0, 500.0),
            32.0
        );
        assert_eq!(
            preserve_scroll_origin_from_top(0.0, 344.0, 648.0, 500.0),
            0.0
        );
    }
}
