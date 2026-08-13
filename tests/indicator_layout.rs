use statlet::indicator::{
    measure_stable_layout, measure_stable_layout_with_prefixes, TextMeasurer,
};

struct FakeMeasurer;

impl TextMeasurer for FakeMeasurer {
    fn width(&self, text: &str) -> f64 {
        text.chars()
            .map(|character| if character == '1' { 3.0 } else { 7.0 })
            .sum()
    }

    fn content_height(&self) -> f64 {
        18.0
    }
}

#[test]
fn stable_layout_uses_the_widest_value_from_zero_through_one_hundred() {
    let layout = measure_stable_layout(&FakeMeasurer, true, 40.0);

    assert_eq!(layout.cpu_width, 38.0);
    assert_eq!(layout.ram_width, 38.0);
    assert_eq!(layout.base_width(), 38.0);
}

#[test]
fn visible_metric_values_share_the_v1_three_digit_right_edge() {
    let layout = measure_stable_layout(&FakeMeasurer, true, 40.0);

    for value in ["0%", "9%", "10%", "99%", "100%"] {
        let origin = layout.value_origin(&FakeMeasurer, layout.cpu_width, value);
        assert_eq!(origin + FakeMeasurer.width(value), layout.cpu_width);
    }
}

#[test]
fn hiding_labels_removes_their_width_from_the_stable_measurement() {
    let layout = measure_stable_layout(&FakeMeasurer, false, 40.0);

    assert_eq!(layout.cpu_width, 24.0);
    assert_eq!(layout.ram_width, 24.0);
}

#[test]
fn custom_label_prefixes_and_spacing_are_measured_before_rendering() {
    let layout = measure_stable_layout_with_prefixes(
        &FakeMeasurer,
        Some("CPU uso  "),
        Some("Memória  "),
        40.0,
    );

    assert_eq!(layout.cpu_width, FakeMeasurer.width("CPU uso  100%"));
    assert_eq!(layout.ram_width, FakeMeasurer.width("Memória  100%"));
    assert_eq!(layout.base_width(), layout.ram_width);
}

#[test]
fn hidden_label_values_keep_the_same_three_digit_right_alignment() {
    let layout = measure_stable_layout(&FakeMeasurer, false, 40.0);

    for value in ["0%", "9%", "10%", "99%", "100%"] {
        let origin = layout.value_origin(&FakeMeasurer, layout.base_width(), value);
        assert_eq!(origin + FakeMeasurer.width(value), layout.base_width());
    }
}

#[test]
fn badge_width_is_added_only_while_the_badge_exists() {
    let layout = measure_stable_layout(&FakeMeasurer, true, 40.0);

    assert_eq!(layout.width_for_badge(None), 38.0);
    assert_eq!(layout.width_for_badge(Some(" !")), 52.0);
    assert_eq!(layout.width_for_badge(Some(" ×")), 52.0);
    assert_eq!(layout.base_width(), 38.0);
}

struct BoundaryMeasurer {
    width: f64,
    height: f64,
}

impl TextMeasurer for BoundaryMeasurer {
    fn width(&self, _text: &str) -> f64 {
        self.width
    }

    fn content_height(&self) -> f64 {
        self.height
    }
}

#[test]
fn diagnostics_warn_only_above_the_curated_boundaries() {
    let at_limits = measure_stable_layout(
        &BoundaryMeasurer {
            width: 80.0,
            height: 22.0,
        },
        true,
        40.0,
    );
    let above_limits = measure_stable_layout(
        &BoundaryMeasurer {
            width: 80.1,
            height: 22.1,
        },
        true,
        40.0,
    );

    assert!(!at_limits.diagnostics.exceeds_menu_bar_height);
    assert!(!at_limits.diagnostics.exceeds_curated_width);
    assert!(above_limits.diagnostics.exceeds_menu_bar_height);
    assert!(above_limits.diagnostics.exceeds_curated_width);
}
