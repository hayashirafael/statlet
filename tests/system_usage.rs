use std::time::Duration;

use statlet::core::MemoryPressure;
use statlet::system_usage::{
    GpuReading, GraphNavigation, GraphNavigationCommand, SystemUsageAccessibilityCoordinator,
    SystemUsageAccessibilityState, SystemUsageSection,
};

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
        statlet::system_usage::UsagePoint {
            observed_at: Duration::from_secs(10),
            value: Some(10.0),
        },
        statlet::system_usage::UsagePoint {
            observed_at: Duration::from_secs(12),
            value: None,
        },
        statlet::system_usage::UsagePoint {
            observed_at: Duration::from_secs(14),
            value: Some(30.0),
        },
        statlet::system_usage::UsagePoint {
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
        statlet::system_usage::UsagePoint {
            observed_at: Duration::from_secs(10),
            value: Some(10.0),
        },
        statlet::system_usage::UsagePoint {
            observed_at: Duration::from_secs(12),
            value: None,
        },
        statlet::system_usage::UsagePoint {
            observed_at: Duration::from_secs(14),
            value: Some(30.0),
        },
    ];
    let window_end = Duration::from_secs(14);

    let valid = statlet::system_usage::graph_pointer_selection(
        &points,
        statlet::system_usage::history_x_position(Duration::from_secs(10), window_end),
    )
    .unwrap();
    assert_eq!(valid.index, 0);
    assert_eq!(valid.accessibility_value, "10%, há 4 segundos");
    assert!(!valid.should_notify_accessibility);
    assert_eq!(
        statlet::system_usage::graph_pointer_selection(
            &points,
            statlet::system_usage::history_x_position(Duration::from_secs(12), window_end),
        ),
        None
    );
}

mod session_contract {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::time::Duration;

    use statlet::core::MemoryPressure;
    use statlet::metrics::MemoryReading;
    use statlet::system_usage::{
        GpuReading, GpuSampleOutcome, ProcessMemory, ProcessSampleOutcome, SystemUsageSection,
    };
    use statlet::system_usage::{
        ProcessSampleRequest, ProcessStart, SamplingCycle, SurfaceObservation, SystemUsageCause,
        SystemUsagePresentation, SystemUsageSampling, SystemUsageSession, SystemUsageSurface,
    };

    #[derive(Default)]
    struct RecordingSystemUsageSampling {
        memory_outcomes: VecDeque<Result<MemoryReading, ()>>,
        gpu_outcomes: VecDeque<GpuSampleOutcome>,
        process_starts: VecDeque<ProcessStart>,
        memory_cycles: Vec<SamplingCycle>,
        cached_system: Option<(SamplingCycle, Result<MemoryReading, ()>)>,
        system_os_reads: usize,
        gpu_reads: usize,
        process_requests: Vec<ProcessSampleRequest>,
    }

    impl RecordingSystemUsageSampling {
        fn system_read(&mut self, cycle: SamplingCycle) -> Result<MemoryReading, ()> {
            if let Some((cached_cycle, outcome)) = self.cached_system {
                if cached_cycle == cycle {
                    return outcome;
                }
            }
            self.system_os_reads += 1;
            let outcome = self
                .memory_outcomes
                .pop_front()
                .unwrap_or_else(|| Ok(memory(50.0)));
            self.cached_system = Some((cycle, outcome));
            outcome
        }

        fn compact(&mut self, cycle: SamplingCycle) -> Result<MemoryReading, ()> {
            self.system_read(cycle)
        }
    }

    impl SystemUsageSampling for RecordingSystemUsageSampling {
        fn memory(&mut self, cycle: SamplingCycle) -> Result<MemoryReading, ()> {
            self.memory_cycles.push(cycle);
            self.system_read(cycle)
        }

        fn gpu(&mut self) -> GpuSampleOutcome {
            self.gpu_reads += 1;
            self.gpu_outcomes
                .pop_front()
                .unwrap_or(GpuSampleOutcome::Unavailable)
        }

        fn start_processes(&mut self, request: ProcessSampleRequest) -> ProcessStart {
            self.process_requests.push(request);
            self.process_starts
                .pop_front()
                .unwrap_or(ProcessStart::Started)
        }
    }

    #[derive(Default)]
    struct RecordingSystemUsageSurface {
        observation: Cell<SurfaceObservation>,
        presentations: Vec<SystemUsagePresentation>,
    }

    impl RecordingSystemUsageSurface {
        fn set_observation(&self, observation: SurfaceObservation) {
            self.observation.set(observation);
        }
    }

    impl SystemUsageSurface for RecordingSystemUsageSurface {
        fn observe(&self) -> SurfaceObservation {
            self.observation.get()
        }

        fn apply(&mut self, presentation: SystemUsagePresentation) {
            self.presentations.push(presentation);
        }
    }

    fn memory(percent: f64) -> MemoryReading {
        let total_bytes = 1_000;
        let used_bytes = (percent * 10.0) as u64;
        MemoryReading {
            app_bytes: used_bytes,
            wired_bytes: 0,
            compressed_bytes: 0,
            used_bytes,
            total_bytes,
            available_bytes: total_bytes - used_bytes,
            cached_bytes: 0,
            swap_used_bytes: 0,
            percent,
            pressure: MemoryPressure::Normal,
        }
    }

    fn gpu(percent: f64) -> GpuSampleOutcome {
        GpuSampleOutcome::Available(
            GpuReading::normalized(percent, None, None, None, None, None).unwrap(),
        )
    }

    fn open(
        session: &mut SystemUsageSession,
        sampling: &mut RecordingSystemUsageSampling,
        surface: &mut RecordingSystemUsageSurface,
        now: Duration,
        native_visibility_epoch: u64,
    ) {
        surface.set_observation(SurfaceObservation {
            visible: true,
            native_visibility_epoch,
            process_interaction_active: false,
        });
        session.advance(SystemUsageCause::SurfaceChanged, now, sampling, surface);
    }

    #[test]
    fn closed_session_is_idle_and_open_session_renders_then_samples_immediately() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        let mut surface = RecordingSystemUsageSurface::default();

        session.advance(
            SystemUsageCause::SurfaceChanged,
            Duration::ZERO,
            &mut sampling,
            &mut surface,
        );

        assert_eq!(session.next_deadline(), None);
        assert!(sampling.memory_cycles.is_empty());
        assert_eq!(sampling.gpu_reads, 0);
        assert!(sampling.process_requests.is_empty());
        assert!(surface.presentations.is_empty());

        surface.set_observation(SurfaceObservation {
            visible: true,
            native_visibility_epoch: 1,
            process_interaction_active: false,
        });
        session.advance(
            SystemUsageCause::SurfaceChanged,
            Duration::from_secs(10),
            &mut sampling,
            &mut surface,
        );

        assert_eq!(session.next_deadline(), Some(Duration::from_secs(10)));
        assert_eq!(surface.presentations.len(), 1);

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(7)),
            Duration::from_secs(10),
            &mut sampling,
            &mut surface,
        );

        assert_eq!(sampling.memory_cycles, vec![SamplingCycle::new(7)]);
        assert_eq!(sampling.gpu_reads, 1);
        assert_eq!(sampling.process_requests.len(), 1);
        assert_eq!(session.next_deadline(), Some(Duration::from_secs(12)));
        assert_eq!(surface.presentations.len(), 2);
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .primary_value,
            "50% em uso"
        );
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .process_status
                .message(),
            "Coletando detalhes por processo…"
        );
    }

    #[test]
    fn effective_ticks_use_two_and_six_second_cadences_without_catch_up() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        let mut surface = RecordingSystemUsageSurface::default();
        open(
            &mut session,
            &mut sampling,
            &mut surface,
            Duration::from_secs(10),
            1,
        );

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::from_secs(10),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.memory_cycles.len(), 1);
        assert_eq!(sampling.process_requests.len(), 1);
        let applies_after_first_tick = surface.presentations.len();

        session.advance(
            SystemUsageCause::SurfaceChanged,
            Duration::from_secs(10),
            &mut sampling,
            &mut surface,
        );
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(2)),
            Duration::from_secs(11),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.memory_cycles.len(), 1);
        assert_eq!(surface.presentations.len(), applies_after_first_tick);

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(3)),
            Duration::from_secs(100),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.memory_cycles.len(), 2);
        assert_eq!(sampling.process_requests.len(), 1);
        assert_eq!(session.next_deadline(), Some(Duration::from_secs(102)));

        let first = sampling.process_requests[0].clone();
        session.advance(
            SystemUsageCause::ProcessesFinished(first.finish(ProcessSampleOutcome::Cancelled)),
            Duration::from_secs(101),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(session.next_deadline(), Some(Duration::from_secs(102)));
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(4)),
            Duration::from_secs(102),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.process_requests.len(), 2);
    }

    #[test]
    fn process_eligibility_coalesces_with_the_system_usage_sampling_wakeup() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        let mut surface = RecordingSystemUsageSurface::default();
        open(
            &mut session,
            &mut sampling,
            &mut surface,
            Duration::from_secs(10),
            1,
        );

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::from_secs(10),
            &mut sampling,
            &mut surface,
        );
        let first = sampling.process_requests[0].clone();
        session.advance(
            SystemUsageCause::ProcessesFinished(first.finish(ProcessSampleOutcome::Cancelled)),
            Duration::from_millis(10_100),
            &mut sampling,
            &mut surface,
        );

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(2)),
            Duration::from_millis(12_100),
            &mut sampling,
            &mut surface,
        );
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(3)),
            Duration::from_millis(14_200),
            &mut sampling,
            &mut surface,
        );

        assert_eq!(session.next_deadline(), Some(Duration::from_millis(16_200)));
        assert_eq!(sampling.process_requests.len(), 1);

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(4)),
            Duration::from_millis(16_200),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.process_requests.len(), 2);
    }

    #[test]
    fn process_sampling_is_single_flight_across_close_and_reopen() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::ZERO,
            &mut sampling,
            &mut surface,
        );
        let old_request = sampling.process_requests[0].clone();

        surface.set_observation(SurfaceObservation {
            visible: false,
            native_visibility_epoch: 2,
            process_interaction_active: false,
        });
        session.advance(
            SystemUsageCause::SurfaceChanged,
            Duration::from_secs(1),
            &mut sampling,
            &mut surface,
        );
        assert!(old_request.cancellation().is_cancelled());
        assert_eq!(session.next_deadline(), None);

        open(
            &mut session,
            &mut sampling,
            &mut surface,
            Duration::from_secs(2),
            3,
        );
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(2)),
            Duration::from_secs(2),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.process_requests.len(), 1);
        let applies_before_stale_completion = surface.presentations.len();

        session.advance(
            SystemUsageCause::ProcessesFinished(old_request.finish(
                ProcessSampleOutcome::Available(vec![ProcessMemory {
                    pid: 1,
                    name: "stale".to_owned(),
                    memory_bytes: 100,
                }]),
            )),
            Duration::from_secs(3),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(surface.presentations.len(), applies_before_stale_completion);
        assert!(surface
            .presentations
            .last()
            .unwrap()
            .view_model
            .process_rows
            .is_empty());

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(3)),
            Duration::from_secs(3),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.process_requests.len(), 2);
    }

    #[test]
    fn failed_process_spawn_retries_only_at_the_normal_six_second_cadence() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        sampling
            .process_starts
            .extend([ProcessStart::Failed, ProcessStart::Started]);
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);

        for (cycle, second) in [(1, 0), (2, 2), (3, 4)] {
            session.advance(
                SystemUsageCause::Wake(SamplingCycle::new(cycle)),
                Duration::from_secs(second),
                &mut sampling,
                &mut surface,
            );
        }
        assert_eq!(sampling.process_requests.len(), 1);
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .process_status,
            statlet::system_usage::ProcessListStatus::Failed
        );

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(4)),
            Duration::from_secs(6),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(sampling.process_requests.len(), 2);
    }

    #[test]
    fn coincident_compact_and_detail_requests_share_one_physical_system_read() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        sampling.memory_outcomes.push_back(Ok(memory(64.0)));
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);
        let cycle = SamplingCycle::new(42);

        let compact = sampling.compact(cycle).unwrap();
        session.advance(
            SystemUsageCause::Wake(cycle),
            Duration::ZERO,
            &mut sampling,
            &mut surface,
        );

        assert_eq!(sampling.system_os_reads, 1);
        assert_eq!(sampling.memory_cycles, vec![cycle]);
        assert_eq!(compact.percent, 64.0);
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .primary_value,
            "64% em uso"
        );
    }

    #[test]
    fn histories_are_bounded_to_one_hundred_fifty_points_and_mark_one_long_gap() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        for index in 0..=160 {
            sampling
                .memory_outcomes
                .push_back(Ok(memory((index % 100) as f64)));
            sampling.gpu_outcomes.push_back(gpu((index % 100) as f64));
        }
        sampling.memory_outcomes.push_back(Ok(memory(75.0)));
        sampling.gpu_outcomes.push_back(gpu(75.0));
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);

        for index in 0..=160 {
            session.advance(
                SystemUsageCause::Wake(SamplingCycle::new(index)),
                Duration::from_secs(index * 2),
                &mut sampling,
                &mut surface,
            );
        }
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(999)),
            Duration::from_secs(400),
            &mut sampling,
            &mut surface,
        );

        let history = &surface.presentations.last().unwrap().view_model.history;
        assert!(history.len() <= 150);
        assert!(history
            .first()
            .is_none_or(|point| point.observed_at >= Duration::from_secs(100)));
        assert_eq!(
            history.iter().filter(|point| point.value.is_none()).count(),
            1
        );

        session.advance(
            SystemUsageCause::SelectSection(SystemUsageSection::Gpu),
            Duration::from_secs(400),
            &mut sampling,
            &mut surface,
        );
        let gpu_history = &surface.presentations.last().unwrap().view_model.history;
        assert!(gpu_history.len() <= 150);
        assert_eq!(
            gpu_history
                .iter()
                .filter(|point| point.value.is_none())
                .count(),
            1
        );
    }

    #[test]
    fn failures_preserve_last_values_as_stale_without_losing_the_session() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        sampling.memory_outcomes.extend([Ok(memory(50.0)), Err(())]);
        sampling
            .gpu_outcomes
            .extend([gpu(30.0), GpuSampleOutcome::Failed]);
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);

        for (cycle, second) in [(1, 0), (2, 2)] {
            session.advance(
                SystemUsageCause::Wake(SamplingCycle::new(cycle)),
                Duration::from_secs(second),
                &mut sampling,
                &mut surface,
            );
        }
        let ram = &surface.presentations.last().unwrap().view_model;
        assert_eq!(ram.primary_value, "50% em uso");
        assert!(ram.status.contains("atualização interrompida"));

        session.advance(
            SystemUsageCause::SelectSection(SystemUsageSection::Gpu),
            Duration::from_secs(2),
            &mut sampling,
            &mut surface,
        );
        let gpu = &surface.presentations.last().unwrap().view_model;
        assert_eq!(gpu.primary_value, "30% de uso");
        assert!(gpu.status.contains("Atualização interrompida"));
        assert!(surface.presentations.last().unwrap().focus_summary);
    }

    #[test]
    fn process_reordering_waits_while_the_process_table_is_being_used() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::ZERO,
            &mut sampling,
            &mut surface,
        );
        let first = sampling.process_requests[0].clone();
        session.advance(
            SystemUsageCause::ProcessesFinished(first.finish(ProcessSampleOutcome::Available(
                vec![ProcessMemory {
                    pid: 1,
                    name: "A".to_owned(),
                    memory_bytes: 100,
                }],
            ))),
            Duration::from_secs(1),
            &mut sampling,
            &mut surface,
        );

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(2)),
            Duration::from_secs(6),
            &mut sampling,
            &mut surface,
        );
        let second = sampling.process_requests[1].clone();
        surface.set_observation(SurfaceObservation {
            visible: true,
            native_visibility_epoch: 1,
            process_interaction_active: true,
        });
        let applies_before_deferred_completion = surface.presentations.len();
        session.advance(
            SystemUsageCause::ProcessesFinished(second.finish(ProcessSampleOutcome::Available(
                vec![ProcessMemory {
                    pid: 2,
                    name: "B".to_owned(),
                    memory_bytes: 200,
                }],
            ))),
            Duration::from_secs(7),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(
            surface.presentations.len(),
            applies_before_deferred_completion
        );
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .process_rows[0]
                .name,
            "A"
        );

        surface.set_observation(SurfaceObservation {
            visible: true,
            native_visibility_epoch: 1,
            process_interaction_active: false,
        });
        session.advance(
            SystemUsageCause::SurfaceChanged,
            Duration::from_secs(8),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .process_rows[0]
                .name,
            "B"
        );
    }

    #[test]
    fn causal_counters_stay_zero_closed_and_within_fixed_bounds_for_sixty_open_seconds() {
        let mut closed = SystemUsageSession::new();
        let mut closed_sampling = RecordingSystemUsageSampling::default();
        let mut closed_surface = RecordingSystemUsageSurface::default();
        for second in (0..=300).step_by(2) {
            closed.advance(
                SystemUsageCause::Wake(SamplingCycle::new(second)),
                Duration::from_secs(second),
                &mut closed_sampling,
                &mut closed_surface,
            );
        }
        assert_eq!(closed.next_deadline(), None);
        assert_eq!(closed_sampling.system_os_reads, 0);
        assert_eq!(closed_sampling.gpu_reads, 0);
        assert!(closed_sampling.process_requests.is_empty());
        assert!(closed_surface.presentations.is_empty());

        let mut open_session = SystemUsageSession::new();
        let mut open_sampling = RecordingSystemUsageSampling::default();
        let mut open_surface = RecordingSystemUsageSurface::default();
        open(
            &mut open_session,
            &mut open_sampling,
            &mut open_surface,
            Duration::ZERO,
            1,
        );
        for second in (0..=60).step_by(2) {
            let requests_before = open_sampling.process_requests.len();
            open_session.advance(
                SystemUsageCause::Wake(SamplingCycle::new(second)),
                Duration::from_secs(second),
                &mut open_sampling,
                &mut open_surface,
            );
            if open_sampling.process_requests.len() > requests_before {
                let request = open_sampling.process_requests.last().unwrap().clone();
                open_session.advance(
                    SystemUsageCause::ProcessesFinished(
                        request.finish(ProcessSampleOutcome::Available(Vec::new())),
                    ),
                    Duration::from_secs(second),
                    &mut open_sampling,
                    &mut open_surface,
                );
            }
        }
        assert!(open_sampling.system_os_reads <= 31);
        assert!(open_sampling.gpu_reads <= 31);
        assert!(open_sampling.process_requests.len() <= 11);
    }

    #[test]
    fn ram_survives_reopen_while_gpu_and_process_rows_restart_with_the_visible_session() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        sampling.memory_outcomes.push_back(Ok(memory(68.0)));
        sampling.gpu_outcomes.push_back(gpu(37.0));
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::ZERO,
            &mut sampling,
            &mut surface,
        );
        let request = sampling.process_requests[0].clone();
        session.advance(
            SystemUsageCause::ProcessesFinished(request.finish(ProcessSampleOutcome::Available(
                vec![ProcessMemory {
                    pid: 7,
                    name: "Orca".to_owned(),
                    memory_bytes: 300,
                }],
            ))),
            Duration::from_secs(1),
            &mut sampling,
            &mut surface,
        );
        session.advance(
            SystemUsageCause::SelectSection(SystemUsageSection::Gpu),
            Duration::from_secs(1),
            &mut sampling,
            &mut surface,
        );
        assert_eq!(
            surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .primary_value,
            "37% de uso"
        );

        surface.set_observation(SurfaceObservation {
            visible: false,
            native_visibility_epoch: 2,
            process_interaction_active: false,
        });
        session.advance(
            SystemUsageCause::SurfaceChanged,
            Duration::from_secs(2),
            &mut sampling,
            &mut surface,
        );
        open(
            &mut session,
            &mut sampling,
            &mut surface,
            Duration::from_secs(3),
            3,
        );
        let reopened_gpu = &surface.presentations.last().unwrap().view_model;
        assert_eq!(reopened_gpu.primary_value, "—");
        assert_eq!(reopened_gpu.status, "Coletando a primeira leitura…");
        assert!(reopened_gpu.process_rows.is_empty());

        session.advance(
            SystemUsageCause::SelectSection(SystemUsageSection::Ram),
            Duration::from_secs(3),
            &mut sampling,
            &mut surface,
        );
        let reopened_ram = &surface.presentations.last().unwrap().view_model;
        assert_eq!(reopened_ram.primary_value, "68% em uso");
        assert_eq!(reopened_ram.history.len(), 1);
        assert!(reopened_ram.process_rows.is_empty());
    }

    #[test]
    fn process_presentations_are_sorted_bounded_and_preserved_as_stale_on_failure() {
        let mut session = SystemUsageSession::new();
        let mut sampling = RecordingSystemUsageSampling::default();
        let mut surface = RecordingSystemUsageSurface::default();
        open(&mut session, &mut sampling, &mut surface, Duration::ZERO, 1);
        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::ZERO,
            &mut sampling,
            &mut surface,
        );
        let rows = (0..25)
            .map(|pid| ProcessMemory {
                pid,
                name: format!("P{pid}"),
                memory_bytes: u64::from(pid) * 100,
            })
            .collect();
        let first = sampling.process_requests[0].clone();
        session.advance(
            SystemUsageCause::ProcessesFinished(
                first.finish(ProcessSampleOutcome::Available(rows)),
            ),
            Duration::from_secs(1),
            &mut sampling,
            &mut surface,
        );
        let available = &surface.presentations.last().unwrap().view_model;
        assert_eq!(available.process_rows.len(), 20);
        assert_eq!(available.process_rows.first().unwrap().pid, 24);
        assert_eq!(available.process_rows.last().unwrap().pid, 5);

        session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(2)),
            Duration::from_secs(6),
            &mut sampling,
            &mut surface,
        );
        let second = sampling.process_requests[1].clone();
        session.advance(
            SystemUsageCause::ProcessesFinished(second.finish(ProcessSampleOutcome::Failed)),
            Duration::from_secs(7),
            &mut sampling,
            &mut surface,
        );
        let stale = &surface.presentations.last().unwrap().view_model;
        assert_eq!(
            stale.process_status,
            statlet::system_usage::ProcessListStatus::Stale
        );
        assert_eq!(stale.process_rows.len(), 20);
    }

    #[test]
    fn first_memory_failure_and_gpu_names_keep_the_existing_presentation_contract() {
        let mut failed_session = SystemUsageSession::new();
        let mut failed_sampling = RecordingSystemUsageSampling::default();
        failed_sampling.memory_outcomes.push_back(Err(()));
        let mut failed_surface = RecordingSystemUsageSurface::default();
        open(
            &mut failed_session,
            &mut failed_sampling,
            &mut failed_surface,
            Duration::ZERO,
            1,
        );
        failed_session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::ZERO,
            &mut failed_sampling,
            &mut failed_surface,
        );
        let failed = &failed_surface.presentations.last().unwrap().view_model;
        assert_eq!(failed.primary_value, "—");
        assert_eq!(failed.status, "Não foi possível ler a RAM.");
        assert_eq!(
            failed.history_accessibility_label,
            "Histórico de uso de RAM, últimos 5 minutos. Leitura atual indisponível; nenhuma leitura válida; 1 lacuna."
        );

        let mut named_session = SystemUsageSession::new();
        let mut named_sampling = RecordingSystemUsageSampling::default();
        let reading = |name: &str, percent| {
            GpuSampleOutcome::Available(
                GpuReading::normalized(percent, None, None, None, None, Some(name.to_owned()))
                    .unwrap(),
            )
        };
        named_sampling.gpu_outcomes.extend([
            reading("AGXAcceleratorG16G", 30.0),
            reading("Apple M4", 40.0),
        ]);
        let mut named_surface = RecordingSystemUsageSurface::default();
        open(
            &mut named_session,
            &mut named_sampling,
            &mut named_surface,
            Duration::ZERO,
            1,
        );
        named_session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(1)),
            Duration::ZERO,
            &mut named_sampling,
            &mut named_surface,
        );
        named_session.advance(
            SystemUsageCause::SelectSection(SystemUsageSection::Gpu),
            Duration::ZERO,
            &mut named_sampling,
            &mut named_surface,
        );
        assert_eq!(
            named_surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .secondary_value,
            ""
        );
        named_session.advance(
            SystemUsageCause::Wake(SamplingCycle::new(2)),
            Duration::from_secs(2),
            &mut named_sampling,
            &mut named_surface,
        );
        assert_eq!(
            named_surface
                .presentations
                .last()
                .unwrap()
                .view_model
                .secondary_value,
            "Apple M4"
        );
    }
}
