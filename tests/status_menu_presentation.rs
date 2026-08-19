use std::time::Duration;

use statlet::disk::DiskObservation;
use statlet::status_menu::{
    StatusMenuGroupId, StatusMenuItemId, StatusMenuPresentation, StatusReading,
};

fn disk_observation(available_bytes: u64) -> DiskObservation {
    DiskObservation::new(500_000_000_000, available_bytes, Duration::ZERO).unwrap()
}

#[test]
fn cpu_and_ram_share_a_disabled_truthful_status_line() {
    let cases = [
        (
            StatusReading::Current(42),
            StatusReading::Current(68),
            "CPU 42% — leitura atual · RAM 68% — leitura atual",
        ),
        (
            StatusReading::Stale(41),
            StatusReading::Unavailable,
            "CPU 41% — leitura antiga · RAM — leitura indisponível",
        ),
    ];

    for (cpu, ram, expected_title) in cases {
        let menu = StatusMenuPresentation::new(cpu, ram, StatusReading::Unavailable, false);
        let item = menu
            .item(StatusMenuItemId::CpuAndRam)
            .expect("CPU/RAM status line");

        assert_eq!(item.title(), expected_title);
        assert!(!item.is_enabled());
    }
}

#[test]
fn disk_status_is_truthful_and_exists_only_for_the_opt_in_integration() {
    let disabled = StatusMenuPresentation::new(
        StatusReading::Current(42),
        StatusReading::Current(68),
        StatusReading::Current(disk_observation(125_000_000_000)),
        false,
    );
    assert_eq!(disabled.item(StatusMenuItemId::Disk), None);

    let cases = [
        (StatusReading::Unavailable, "Disco — leitura indisponível"),
        (
            StatusReading::Current(disk_observation(125_000_000_000)),
            "Disco 125.0 GB disponíveis — leitura atual",
        ),
        (
            StatusReading::Stale(disk_observation(125_000_000_000)),
            "Disco 125.0 GB disponíveis — leitura antiga",
        ),
    ];

    for (disk, expected_title) in cases {
        let menu = StatusMenuPresentation::new(
            StatusReading::Current(42),
            StatusReading::Current(68),
            disk,
            true,
        );
        let item = menu.item(StatusMenuItemId::Disk).expect("disk status line");

        assert_eq!(item.title(), expected_title);
        assert!(!item.is_enabled());
    }
}

#[test]
fn menu_order_and_separator_groups_are_explicit_domain_data() {
    use StatusMenuGroupId::{Application, Exit, Investigation, Readings};
    use StatusMenuItemId::{
        CpuAndRam, Disk, OpenHistory, OpenPreferences, OpenSystemUsage, Quit, ReviewSpace,
    };

    let menu = StatusMenuPresentation::new(
        StatusReading::Current(42),
        StatusReading::Current(68),
        StatusReading::Current(disk_observation(125_000_000_000)),
        true,
    );

    let groups = menu
        .groups()
        .iter()
        .map(|group| {
            (
                group.id(),
                group
                    .items()
                    .iter()
                    .map(|item| item.id())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        groups,
        vec![
            (Readings, vec![CpuAndRam, Disk]),
            (Investigation, vec![OpenSystemUsage, ReviewSpace]),
            (Application, vec![OpenPreferences, OpenHistory]),
            (Exit, vec![Quit]),
        ]
    );
    assert!(menu.item(ReviewSpace).unwrap().is_enabled());
}

#[test]
fn review_space_remains_discoverable_but_disabled_without_the_integration() {
    let menu = StatusMenuPresentation::new(
        StatusReading::Current(42),
        StatusReading::Current(68),
        StatusReading::Unavailable,
        false,
    );

    let review = menu
        .item(StatusMenuItemId::ReviewSpace)
        .expect("review-space action");
    assert_eq!(review.title(), "Revisar espaço…");
    assert!(!review.is_enabled());
}
