use std::time::Duration;

use statlet::core::MemoryPressure;
use statlet::metrics::{detailed_memory_from_counters, VmCounters};
use statlet::stats::{
    GpuReading, GpuSampleOutcome, ProcessMemory, ProcessSamplingSchedule, SystemUsageModel,
    SystemUsageSection, UsageHistory,
};

#[test]
fn usage_history_keeps_exactly_the_latest_150_points() {
    let mut history = UsageHistory::new();

    for second in 0..151 {
        history.push(Duration::from_secs(second * 2), Some(second as f64));
    }

    assert_eq!(history.points().len(), 150);
    assert_eq!(history.points().front().unwrap().value, Some(1.0));
    assert_eq!(history.points().back().unwrap().value, Some(150.0));
}

#[test]
fn delayed_sample_creates_one_gap_without_catch_up_burst() {
    let mut history = UsageHistory::new();
    history.push(Duration::ZERO, Some(10.0));

    history.push(Duration::from_secs(10), Some(20.0));

    let values = history
        .points()
        .iter()
        .map(|point| point.value)
        .collect::<Vec<_>>();
    assert_eq!(values, vec![Some(10.0), None, Some(20.0)]);
}

fn gpu(percent: f64) -> GpuReading {
    GpuReading {
        utilization_percent: percent,
        renderer_percent: None,
        tiler_percent: None,
        in_use_system_memory_bytes: None,
        allocated_system_memory_bytes: None,
        device_name: Some("Apple GPU".to_owned()),
    }
}

#[test]
fn gpu_history_exists_only_for_the_visible_window_session() {
    let mut model = SystemUsageModel::new();

    model.record_gpu(Duration::ZERO, GpuSampleOutcome::Available(gpu(10.0)));
    assert!(model.gpu_history().points().is_empty());

    model.set_visible(true);
    model.record_gpu(
        Duration::from_secs(2),
        GpuSampleOutcome::Available(gpu(20.0)),
    );
    assert_eq!(model.gpu_history().points().len(), 1);

    model.set_visible(false);
    assert!(model.gpu_history().points().is_empty());
}

#[test]
fn gpu_normalization_rejects_invalid_primary_values_and_omits_invalid_optional_values() {
    assert!(GpuReading::normalized(f64::NAN, None, None, None, None, None).is_none());

    let reading = GpuReading::normalized(
        120.0,
        Some(f64::INFINITY),
        Some(-5.0),
        Some(42),
        None,
        Some("Apple GPU".to_owned()),
    )
    .unwrap();

    assert_eq!(reading.utilization_percent, 100.0);
    assert_eq!(reading.renderer_percent, None);
    assert_eq!(reading.tiler_percent, Some(0.0));
    assert_eq!(reading.in_use_system_memory_bytes, Some(42));
}

#[test]
fn ram_view_model_remains_complete_when_gpu_is_unavailable() {
    let memory = detailed_memory_from_counters(
        16_000_000_000,
        100_000_000,
        VmCounters {
            active: 20,
            inactive: 10,
            speculative: 2,
            wired: 5,
            compressed: 4,
            purgeable: 3,
            external: 6,
        },
        700_000_000,
        MemoryPressure::Warning,
    );
    let mut model = SystemUsageModel::new();
    model.record_memory(Duration::ZERO, Ok(memory));
    model.set_visible(true);
    model.record_gpu(Duration::ZERO, GpuSampleOutcome::Unavailable);

    let ram = model.view_model(SystemUsageSection::Ram);
    assert_eq!(ram.primary_value, "20% em uso");
    assert_eq!(ram.secondary_value, "3,2 GB de 16 GB");
    assert_eq!(ram.status, "△ Pressão em atenção");
    assert_eq!(
        ram.details.iter().map(|row| row.label).collect::<Vec<_>>(),
        vec![
            "Apps",
            "Reservada pelo sistema",
            "Comprimida",
            "Disponível",
            "Cache recuperável",
            "Swap em uso"
        ]
    );

    let gpu = model.view_model(SystemUsageSection::Gpu);
    assert_eq!(gpu.primary_value, "—");
    assert_eq!(gpu.status, "O uso da GPU não está disponível neste Mac.");
}

#[test]
fn top_processes_are_ephemeral_sorted_and_limited_to_twenty() {
    let processes = (0..25)
        .map(|pid| ProcessMemory {
            pid,
            name: format!("process-{pid}"),
            memory_bytes: u64::from(pid) * 1_000_000,
        })
        .collect::<Vec<_>>();
    let mut model = SystemUsageModel::new();

    model.record_processes(Ok(processes.clone()));
    assert!(model
        .view_model(SystemUsageSection::Ram)
        .process_rows
        .is_empty());

    model.set_visible(true);
    model.record_processes(Ok(processes));
    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(view.process_rows.len(), 20);
    assert_eq!(view.process_rows.first().unwrap().pid, 24);
    assert_eq!(view.process_rows.last().unwrap().pid, 5);

    model.set_visible(false);
    assert!(model
        .view_model(SystemUsageSection::Ram)
        .process_rows
        .is_empty());
}

#[test]
fn process_sampling_reuses_ticks_and_never_catches_up() {
    let mut schedule = ProcessSamplingSchedule::new();
    assert!(!schedule.take_due(Duration::ZERO));

    schedule.set_visible(true, Duration::from_secs(10));
    assert!(schedule.take_due(Duration::from_secs(10)));
    assert!(!schedule.take_due(Duration::from_secs(12)));
    assert!(schedule.take_due(Duration::from_secs(14)));
    assert!(schedule.take_due(Duration::from_secs(100)));
    assert!(!schedule.take_due(Duration::from_secs(100)));

    schedule.set_visible(false, Duration::from_secs(100));
    assert!(!schedule.take_due(Duration::from_secs(104)));
    schedule.set_visible(true, Duration::from_secs(110));
    assert!(schedule.take_due(Duration::from_secs(110)));
}

#[test]
fn ram_failure_keeps_the_last_value_marked_as_stale_and_adds_a_gap() {
    let memory = detailed_memory_from_counters(
        1_000,
        100,
        VmCounters {
            active: 2,
            inactive: 1,
            speculative: 0,
            wired: 1,
            compressed: 1,
            purgeable: 0,
            external: 0,
        },
        0,
        MemoryPressure::Normal,
    );
    let mut model = SystemUsageModel::new();
    model.record_memory(Duration::ZERO, Ok(memory));

    model.record_memory(Duration::from_secs(2), Err(()));

    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(view.primary_value, "50% em uso");
    assert_eq!(
        view.status,
        "✓ Pressão normal — atualização interrompida; última leitura preservada"
    );
    assert_eq!(view.history.last().unwrap().value, None);
}

#[test]
fn ram_history_has_one_complete_accessibility_summary() {
    let reading = |used_pages| {
        detailed_memory_from_counters(
            1_000,
            100,
            VmCounters {
                active: used_pages,
                inactive: 0,
                speculative: 0,
                wired: 0,
                compressed: 0,
                purgeable: 0,
                external: 0,
            },
            0,
            MemoryPressure::Normal,
        )
    };
    let mut model = SystemUsageModel::new();
    model.record_memory(Duration::ZERO, Ok(reading(2)));
    model.record_memory(Duration::from_secs(2), Ok(reading(5)));

    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(
        view.history_accessibility_label,
        "Histórico de uso de RAM, últimos 5 minutos. Atual 50%, mínimo 20%, máximo 50%."
    );
}

#[test]
fn gpu_failure_keeps_the_last_value_marked_as_stale() {
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    model.record_gpu(Duration::ZERO, GpuSampleOutcome::Available(gpu(30.0)));

    model.record_gpu(Duration::from_secs(2), GpuSampleOutcome::Failed);

    let view = model.view_model(SystemUsageSection::Gpu);
    assert_eq!(view.primary_value, "30% de uso");
    assert_eq!(
        view.status,
        "Atualização interrompida; última leitura preservada"
    );
    assert_eq!(view.history.last().unwrap().value, None);
}

#[test]
fn out_of_order_history_sample_is_ignored() {
    let mut history = UsageHistory::new();
    history.push(Duration::from_secs(4), Some(40.0));

    history.push(Duration::from_secs(2), Some(20.0));
    history.push(Duration::from_secs(4), Some(99.0));

    assert_eq!(history.points().len(), 1);
    assert_eq!(history.points().back().unwrap().value, Some(40.0));
}
