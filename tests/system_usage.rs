use std::time::Duration;

use statlet::core::MemoryPressure;
use statlet::metrics::{detailed_memory_from_counters, VmCounters};
use statlet::stats::{
    GpuReading, GpuSampleOutcome, GraphNavigation, GraphNavigationCommand, ProcessListStatus,
    ProcessMemory, ProcessSampleCancellation, ProcessSampleCompletion, ProcessSampleOutcome,
    ProcessSamplingSchedule, SystemUsageAccessibilityCoordinator, SystemUsageAccessibilityState,
    SystemUsageModel, SystemUsageRenderCoalescer, SystemUsageSamplingCoordinator,
    SystemUsageSamplingPorts, SystemUsageSection, UsageHistory,
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
fn usage_history_expires_points_older_than_five_minutes_by_timestamp() {
    let mut history = UsageHistory::new();
    history.push(Duration::ZERO, Some(10.0));
    history.push(Duration::from_secs(299), Some(20.0));

    history.push(Duration::from_secs(301), Some(30.0));

    let points = history.points().iter().copied().collect::<Vec<_>>();
    assert!(points
        .iter()
        .all(|point| point.observed_at >= Duration::from_secs(1)));
    assert_eq!(points.last().unwrap().value, Some(30.0));
}

#[test]
fn usage_points_are_positioned_by_elapsed_time_in_the_five_minute_window() {
    let position = |observed_at| {
        statlet::stats::history_x_position(
            Duration::from_secs(observed_at),
            Duration::from_secs(300),
        )
    };

    assert_eq!(position(0), 0.0);
    assert_eq!(position(150), 0.5);
    assert_eq!(position(299), 299.0 / 300.0);
    assert_eq!(position(300), 1.0);
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
fn reopening_detail_preserves_ram_history_and_current_reading() {
    let memory = detailed_memory_from_counters(
        1_000,
        100,
        VmCounters {
            active: 2,
            inactive: 0,
            speculative: 0,
            wired: 0,
            compressed: 0,
            purgeable: 0,
            external: 0,
        },
        0,
        MemoryPressure::Normal,
    );
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    model.record_memory(Duration::ZERO, Ok(memory));

    model.set_visible(false);
    model.set_visible(true);

    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(view.primary_value, "20% em uso");
    assert_eq!(view.status, "✓ Pressão normal");
    assert_eq!(view.history.len(), 1);
    assert_eq!(view.history[0].value, Some(20.0));
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
        ram.accessibility_state,
        SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Warning)
    );
    assert_eq!(
        ram.memory_composition
            .iter()
            .map(|segment| segment.label)
            .collect::<Vec<_>>(),
        vec!["Apps", "Reservada", "Comprimida", "Disponível"]
    );
    assert!(
        (ram.memory_composition
            .iter()
            .map(|segment| segment.fraction)
            .sum::<f64>()
            - 1.0)
            .abs()
            < f64::EPSILON
    );
    assert_eq!(
        ram.memory_composition_accessibility_label,
        "Composição da memória física: Apps 2,3 GB; Reservada 500 MB; Comprimida 400 MB; Disponível 12,8 GB."
    );
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
    assert_eq!(
        gpu.accessibility_state,
        SystemUsageAccessibilityState::Unavailable
    );
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

    model.record_process_sample(
        Duration::ZERO,
        ProcessSampleOutcome::Available(processes.clone()),
        false,
    );
    assert!(model
        .view_model(SystemUsageSection::Ram)
        .process_rows
        .is_empty());

    model.set_visible(true);
    model.record_process_sample(
        Duration::from_secs(1),
        ProcessSampleOutcome::Available(processes),
        false,
    );
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
fn process_list_distinguishes_collecting_unavailable_and_failed_states() {
    let mut model = SystemUsageModel::new();
    model.set_visible(true);

    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_status,
        ProcessListStatus::Collecting
    );

    model.record_process_sample(Duration::ZERO, ProcessSampleOutcome::Unavailable, false);
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_status,
        ProcessListStatus::Unavailable
    );

    model.record_process_sample(Duration::from_secs(6), ProcessSampleOutcome::Failed, false);
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_status,
        ProcessListStatus::Failed
    );
    assert_eq!(
        ProcessListStatus::Unavailable.message(),
        "Detalhes por processo não estão disponíveis nesta leitura."
    );
    assert_eq!(
        ProcessListStatus::Stale.message(),
        "Detalhes por processo desatualizados; última leitura preservada."
    );
}

#[test]
fn failed_process_refresh_marks_existing_rows_stale_instead_of_erasing_them() {
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    model.record_process_sample(
        Duration::ZERO,
        ProcessSampleOutcome::Available(vec![ProcessMemory {
            pid: 42,
            name: "Safari".to_owned(),
            memory_bytes: 512_000_000,
        }]),
        false,
    );

    model.record_process_sample(Duration::from_secs(6), ProcessSampleOutcome::Failed, false);

    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(view.process_status, ProcessListStatus::Stale);
    assert_eq!(view.process_rows.len(), 1);
    assert_eq!(view.process_rows[0].pid, 42);
}

#[test]
fn process_reordering_waits_for_focus_release_but_never_longer_than_fifteen_seconds() {
    let row = |pid, memory_bytes| ProcessMemory {
        pid,
        name: format!("process-{pid}"),
        memory_bytes,
    };
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    model.record_process_sample(
        Duration::ZERO,
        ProcessSampleOutcome::Available(vec![row(1, 200), row(2, 100)]),
        false,
    );

    model.record_process_sample(
        Duration::from_secs(6),
        ProcessSampleOutcome::Available(vec![row(1, 100), row(2, 300)]),
        true,
    );
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_rows[0].pid,
        1
    );

    model.apply_deferred_processes(Duration::from_secs(20), true);
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_rows[0].pid,
        1
    );

    model.apply_deferred_processes(Duration::from_secs(21), true);
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_rows[0].pid,
        2
    );

    model.record_process_sample(
        Duration::from_secs(27),
        ProcessSampleOutcome::Available(vec![row(1, 400), row(2, 100)]),
        true,
    );
    model.apply_deferred_processes(Duration::from_secs(28), false);
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_rows[0].pid,
        1
    );
}

#[test]
fn system_usage_rendering_suppresses_duplicate_models_and_accepts_each_change_once() {
    let mut model = SystemUsageModel::new();
    let mut coalescer = SystemUsageRenderCoalescer::new();
    let collecting = model.view_model(SystemUsageSection::Ram);

    assert!(coalescer.take_changed(&collecting));
    assert!(!coalescer.take_changed(&collecting));

    model.record_memory(Duration::ZERO, Err(()));
    let failed = model.view_model(SystemUsageSection::Ram);
    assert!(coalescer.take_changed(&failed));
    assert!(!coalescer.take_changed(&failed));

    coalescer.reset();
    assert!(coalescer.take_changed(&failed));
}

#[test]
fn accessibility_transitions_are_coalesced_and_human_section_focus_is_consumed_once() {
    let mut coordinator = SystemUsageAccessibilityCoordinator::new();

    let initial = coordinator.observe(
        SystemUsageSection::Ram,
        SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Normal),
    );
    assert_eq!(initial.announcement, None);
    assert!(!initial.focus_summary);
    assert_eq!(
        coordinator
            .observe(
                SystemUsageSection::Ram,
                SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Normal),
            )
            .announcement,
        None
    );

    let mut pressure_update = coordinator.observe(
        SystemUsageSection::Ram,
        SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Warning),
    );
    pressure_update
        .include_announcement("O processo selecionado terminou; seleção movida para Orca.");
    assert_eq!(
        pressure_update.announcement,
        Some(
            "Pressão da memória em atenção. O processo selecionado terminou; seleção movida para Orca."
                .to_owned()
        )
    );
    assert_eq!(
        coordinator
            .observe(
                SystemUsageSection::Ram,
                SystemUsageAccessibilityState::Stale,
            )
            .announcement,
        Some("Atualização da RAM interrompida.".to_owned())
    );
    assert_eq!(
        coordinator
            .observe(
                SystemUsageSection::Ram,
                SystemUsageAccessibilityState::Stale,
            )
            .announcement,
        None
    );
    assert_eq!(
        coordinator
            .observe(
                SystemUsageSection::Ram,
                SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Warning),
            )
            .announcement,
        Some("Leitura de RAM restabelecida.".to_owned())
    );

    coordinator.request_summary_focus_after_user_switch(SystemUsageSection::Gpu);
    assert!(
        !coordinator
            .observe(
                SystemUsageSection::Ram,
                SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Warning),
            )
            .focus_summary
    );
    assert!(
        coordinator
            .observe(
                SystemUsageSection::Gpu,
                SystemUsageAccessibilityState::Unavailable,
            )
            .focus_summary
    );
    assert!(
        !coordinator
            .observe(
                SystemUsageSection::Gpu,
                SystemUsageAccessibilityState::Unavailable,
            )
            .focus_summary
    );
}

#[test]
fn ram_recovery_coalesces_a_pressure_change_that_happened_during_failure() {
    for (recovered_pressure, expected) in [
        (
            MemoryPressure::Warning,
            "Leitura de RAM restabelecida. Pressão da memória em atenção.",
        ),
        (
            MemoryPressure::Critical,
            "Leitura de RAM restabelecida. Pressão da memória crítica.",
        ),
    ] {
        let mut coordinator = SystemUsageAccessibilityCoordinator::new();
        coordinator.observe(
            SystemUsageSection::Ram,
            SystemUsageAccessibilityState::MemoryPressure(MemoryPressure::Normal),
        );
        coordinator.observe(
            SystemUsageSection::Ram,
            SystemUsageAccessibilityState::Stale,
        );

        assert_eq!(
            coordinator
                .observe(
                    SystemUsageSection::Ram,
                    SystemUsageAccessibilityState::MemoryPressure(recovered_pressure),
                )
                .announcement
                .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn graph_keyboard_navigation_skips_gaps_and_exposes_selected_value_and_time() {
    let points = vec![
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(10),
            value: Some(10.0),
        },
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(12),
            value: None,
        },
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(14),
            value: Some(30.0),
        },
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(16),
            value: Some(40.0),
        },
    ];
    let mut navigation = GraphNavigation::new();

    let latest = navigation.update_points(&points).unwrap();
    assert_eq!(latest.index, 3);
    assert_eq!(latest.accessibility_value, "40%, agora");
    assert!(!latest.should_notify_accessibility);

    let previous = navigation
        .move_selection(&points, GraphNavigationCommand::Previous)
        .unwrap();
    assert_eq!(previous.index, 2);
    assert_eq!(previous.accessibility_value, "30%, há 2 segundos");
    assert!(previous.should_notify_accessibility);

    let first = navigation
        .move_selection(&points, GraphNavigationCommand::First)
        .unwrap();
    assert_eq!(first.index, 0);
    assert_eq!(first.accessibility_value, "10%, há 6 segundos");
    assert!(first.should_notify_accessibility);

    let unchanged_first = navigation
        .move_selection(&points, GraphNavigationCommand::First)
        .unwrap();
    assert!(!unchanged_first.should_notify_accessibility);

    let next = navigation
        .move_selection(&points, GraphNavigationCommand::Next)
        .unwrap();
    assert_eq!(next.index, 2);

    let last = navigation
        .move_selection(&points, GraphNavigationCommand::Last)
        .unwrap();
    assert_eq!(last.index, 3);
}

#[test]
fn graph_pointer_inspection_uses_elapsed_position_without_crossing_gaps() {
    let points = vec![
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(10),
            value: Some(10.0),
        },
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(12),
            value: None,
        },
        statlet::stats::UsagePoint {
            observed_at: Duration::from_secs(14),
            value: Some(30.0),
        },
    ];
    let window_end = Duration::from_secs(14);

    let valid = statlet::stats::graph_pointer_selection(
        &points,
        statlet::stats::history_x_position(Duration::from_secs(10), window_end),
    )
    .unwrap();
    assert_eq!(valid.index, 0);
    assert_eq!(valid.accessibility_value, "10%, há 4 segundos");
    assert!(!valid.should_notify_accessibility);
    assert_eq!(
        statlet::stats::graph_pointer_selection(
            &points,
            statlet::stats::history_x_position(Duration::from_secs(12), window_end),
        ),
        None
    );
}

#[test]
fn process_sampling_reuses_ticks_and_never_catches_up() {
    let mut schedule = ProcessSamplingSchedule::new();
    assert!(!schedule.take_due(Duration::ZERO));

    schedule.set_visible(true, Duration::from_secs(10));
    assert!(schedule.take_due(Duration::from_secs(10)));
    assert!(!schedule.take_due(Duration::from_secs(12)));
    assert!(!schedule.take_due(Duration::from_secs(14)));
    assert!(schedule.take_due(Duration::from_secs(16)));
    assert!(schedule.take_due(Duration::from_secs(100)));
    assert!(!schedule.take_due(Duration::from_secs(100)));

    schedule.set_visible(false, Duration::from_secs(100));
    assert!(!schedule.take_due(Duration::from_secs(104)));
    schedule.set_visible(true, Duration::from_secs(110));
    assert!(schedule.take_due(Duration::from_secs(110)));
}

#[derive(Default)]
struct CountingSamplingPorts {
    gpu_calls: usize,
    process_calls: Vec<u64>,
    process_cancellations: Vec<ProcessSampleCancellation>,
}

impl SystemUsageSamplingPorts for CountingSamplingPorts {
    fn sample_gpu(&mut self) -> GpuSampleOutcome {
        self.gpu_calls += 1;
        GpuSampleOutcome::Unavailable
    }

    fn request_process_sample(
        &mut self,
        generation: u64,
        cancellation: ProcessSampleCancellation,
    ) -> bool {
        self.process_calls.push(generation);
        self.process_cancellations.push(cancellation);
        true
    }
}

#[test]
fn closed_detail_sampling_causally_prevents_gpu_and_process_calls_without_catch_up() {
    let mut coordinator = SystemUsageSamplingCoordinator::new();
    let mut ports = CountingSamplingPorts::default();

    assert_eq!(
        coordinator.collect_if_visible(Duration::ZERO, false, &mut ports),
        None
    );
    assert_eq!(ports.gpu_calls, 0);
    assert!(ports.process_calls.is_empty());

    assert_eq!(
        coordinator.collect_if_visible(Duration::from_secs(10), true, &mut ports),
        Some(GpuSampleOutcome::Unavailable)
    );
    assert_eq!(ports.gpu_calls, 1);
    assert_eq!(ports.process_calls.len(), 1);
    assert!(!ports.process_cancellations[0].is_cancelled());

    assert_eq!(
        coordinator.collect_if_visible(Duration::from_secs(100), false, &mut ports),
        None
    );
    assert!(ports.process_cancellations[0].is_cancelled());
    assert_eq!(
        coordinator.collect_if_visible(Duration::from_secs(200), false, &mut ports),
        None
    );
    assert_eq!(ports.gpu_calls, 1);
    assert_eq!(ports.process_calls.len(), 1);

    coordinator.collect_if_visible(Duration::from_secs(300), true, &mut ports);
    assert_eq!(ports.gpu_calls, 2);
    assert_eq!(ports.process_calls.len(), 1);
    assert!(!coordinator.accept_process_sample(ports.process_calls[0]));
    coordinator.collect_if_visible(Duration::from_secs(302), true, &mut ports);
    assert_eq!(ports.process_calls.len(), 2);
    assert_ne!(ports.process_calls[0], ports.process_calls[1]);
}

#[test]
fn process_results_from_a_closed_window_generation_are_ignored() {
    let mut coordinator = SystemUsageSamplingCoordinator::new();
    let mut ports = CountingSamplingPorts::default();
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    coordinator.collect_if_visible(Duration::ZERO, true, &mut ports);
    let stale_generation = ports.process_calls[0];

    model.set_visible(false);
    coordinator.collect_if_visible(Duration::from_secs(2), false, &mut ports);
    model.set_visible(true);
    coordinator.collect_if_visible(Duration::from_secs(4), true, &mut ports);
    assert_eq!(ports.process_calls.len(), 1);

    assert!(!coordinator.record_processes_if_current(
        ProcessSampleCompletion {
            observed_at: Duration::from_secs(4),
            live_visible: true,
            interaction_active: false,
            request_visibility_generation: 0,
            live_visibility_generation: 0,
            generation: stale_generation,
            outcome: ProcessSampleOutcome::Available(vec![ProcessMemory {
                pid: 1,
                name: "stale".to_owned(),
                memory_bytes: 100,
            }]),
        },
        &mut model,
    ));
    assert!(model
        .view_model(SystemUsageSection::Ram)
        .process_rows
        .is_empty());

    coordinator.collect_if_visible(Duration::from_secs(6), true, &mut ports);
    assert_eq!(ports.process_calls.len(), 2);
    let current_generation = ports.process_calls[1];

    assert!(coordinator.record_processes_if_current(
        ProcessSampleCompletion {
            observed_at: Duration::from_secs(6),
            live_visible: true,
            interaction_active: false,
            request_visibility_generation: 0,
            live_visibility_generation: 0,
            generation: current_generation,
            outcome: ProcessSampleOutcome::Available(vec![ProcessMemory {
                pid: 2,
                name: "current".to_owned(),
                memory_bytes: 200,
            }]),
        },
        &mut model,
    ));
    assert_eq!(
        model.view_model(SystemUsageSection::Ram).process_rows[0].name,
        "current"
    );
}

#[test]
fn process_result_is_rejected_when_live_visibility_closed_before_polling_observed_it() {
    let mut coordinator = SystemUsageSamplingCoordinator::new();
    let mut ports = CountingSamplingPorts::default();
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    coordinator.collect_if_visible(Duration::ZERO, true, &mut ports);
    let generation = ports.process_calls[0];

    assert!(!coordinator.record_processes_if_current(
        ProcessSampleCompletion {
            observed_at: Duration::from_secs(1),
            live_visible: false,
            interaction_active: false,
            request_visibility_generation: 0,
            live_visibility_generation: 0,
            generation,
            outcome: ProcessSampleOutcome::Available(vec![ProcessMemory {
                pid: 1,
                name: "stale".to_owned(),
                memory_bytes: 100,
            }]),
        },
        &mut model,
    ));
    assert!(model
        .view_model(SystemUsageSection::Ram)
        .process_rows
        .is_empty());
}

#[test]
fn process_result_is_rejected_after_a_quick_close_and_reopen_visibility_epoch() {
    let mut coordinator = SystemUsageSamplingCoordinator::new();
    let mut ports = CountingSamplingPorts::default();
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    coordinator.collect_if_visible(Duration::ZERO, true, &mut ports);
    let generation = ports.process_calls[0];

    assert!(!coordinator.record_processes_if_current(
        ProcessSampleCompletion {
            observed_at: Duration::from_secs(1),
            live_visible: true,
            interaction_active: false,
            request_visibility_generation: 1,
            live_visibility_generation: 3,
            generation,
            outcome: ProcessSampleOutcome::Available(vec![ProcessMemory {
                pid: 1,
                name: "stale".to_owned(),
                memory_bytes: 100,
            }]),
        },
        &mut model,
    ));
    assert!(model
        .view_model(SystemUsageSection::Ram)
        .process_rows
        .is_empty());
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
        "✓ Pressão normal — atualização interrompida; última leitura há 2 s"
    );
    assert_eq!(view.history.last().unwrap().value, None);
    assert_eq!(
        view.accessibility_state,
        SystemUsageAccessibilityState::Stale
    );
}

#[test]
fn initial_ram_failure_is_explicit_instead_of_remaining_collecting() {
    let mut model = SystemUsageModel::new();

    model.record_memory(Duration::ZERO, Err(()));

    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(view.primary_value, "—");
    assert_eq!(view.status, "Não foi possível ler a RAM.");
    assert_eq!(view.history.last().unwrap().value, None);
    assert_eq!(
        view.history_accessibility_label,
        "Histórico de uso de RAM, últimos 5 minutos. Leitura atual indisponível; nenhuma leitura válida; 1 lacuna."
    );
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
fn history_accessibility_marks_a_failed_current_sample_as_stale_and_reports_gaps() {
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
    model.record_memory(Duration::from_secs(4), Err(()));

    let view = model.view_model(SystemUsageSection::Ram);
    assert_eq!(
        view.history_accessibility_label,
        "Histórico de uso de RAM, últimos 5 minutos. Leitura atual indisponível; última válida 50%, mínimo 20%, máximo 50%; 1 lacuna."
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
        "Atualização interrompida; última leitura há 2 s"
    );
    assert_eq!(view.history.last().unwrap().value, None);

    model.record_gpu(Duration::from_secs(10), GpuSampleOutcome::Failed);
    assert_eq!(
        model.view_model(SystemUsageSection::Gpu).status,
        "Atualização interrompida; última leitura há 10 s"
    );
}

#[test]
fn gpu_secondary_value_omits_internal_agx_names_but_keeps_human_models() {
    let mut model = SystemUsageModel::new();
    model.set_visible(true);
    let mut internal = gpu(30.0);
    internal.device_name = Some("AGXAcceleratorG16G".to_owned());
    model.record_gpu(Duration::ZERO, GpuSampleOutcome::Available(internal));
    assert_eq!(
        model.view_model(SystemUsageSection::Gpu).secondary_value,
        ""
    );

    let mut human = gpu(40.0);
    human.device_name = Some("Apple M4".to_owned());
    model.record_gpu(Duration::from_secs(2), GpuSampleOutcome::Available(human));
    assert_eq!(
        model.view_model(SystemUsageSection::Gpu).secondary_value,
        "Apple M4"
    );
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
