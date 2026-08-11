use statlet::core::{AppEvent, MemoryPressure, MetricSeverity, StatletCore, SystemSnapshot};

#[test]
fn metrics_sample_becomes_the_compact_accessible_status() {
    let mut app = StatletCore::new();

    app.handle(AppEvent::MetricsSample(SystemSnapshot {
        cpu_percent: 39.4,
        ram_percent: 72.6,
        memory_pressure: MemoryPressure::Normal,
    }));
    let state = app.state();

    assert_eq!(state.status.top.label, "C");
    assert_eq!(state.status.top.value, " 39%");
    assert_eq!(state.status.top.severity, MetricSeverity::Good);
    assert_eq!(state.status.bottom.label, "R");
    assert_eq!(state.status.bottom.value, " 73%");
    assert_eq!(state.status.bottom.severity, MetricSeverity::Good);
    assert_eq!(
        state.status.accessibility_label,
        "CPU 39%, RAM 73%, pressão de memória normal"
    );
}

#[test]
fn cpu_thresholds_change_only_the_value_severity() {
    let cases = [
        (39.9, MetricSeverity::Good),
        (40.0, MetricSeverity::Warning),
        (69.9, MetricSeverity::Warning),
        (70.0, MetricSeverity::Critical),
    ];

    for (cpu_percent, expected) in cases {
        let mut app = StatletCore::new();
        app.handle(AppEvent::MetricsSample(SystemSnapshot {
            cpu_percent,
            ram_percent: 50.0,
            memory_pressure: MemoryPressure::Normal,
        }));
        let state = app.state();
        assert_eq!(state.status.top.severity, expected);
    }
}

#[test]
fn ram_color_follows_pressure_instead_of_percentage() {
    let cases = [
        (99.0, MemoryPressure::Normal, MetricSeverity::Good),
        (20.0, MemoryPressure::Warning, MetricSeverity::Warning),
        (20.0, MemoryPressure::Critical, MetricSeverity::Critical),
    ];

    for (ram_percent, memory_pressure, expected) in cases {
        let mut app = StatletCore::new();
        app.handle(AppEvent::MetricsSample(SystemSnapshot {
            cpu_percent: 10.0,
            ram_percent,
            memory_pressure,
        }));
        let state = app.state();
        assert_eq!(state.status.bottom.severity, expected);
    }
}

#[test]
fn percentage_fields_keep_the_same_width() {
    let mut app = StatletCore::new();

    for percent in [0.0, 9.0, 10.0, 99.0, 100.0] {
        app.handle(AppEvent::MetricsSample(SystemSnapshot {
            cpu_percent: percent,
            ram_percent: percent,
            memory_pressure: MemoryPressure::Normal,
        }));
        let state = app.state();
        assert_eq!(state.status.top.value.chars().count(), 4);
        assert_eq!(state.status.bottom.value.chars().count(), 4);
    }
}
