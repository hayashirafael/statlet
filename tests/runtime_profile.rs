use std::path::PathBuf;

use statlet::runtime_profile::{
    BundleProfileMetadata, RuntimeProfile, StorageOverrides, PRODUCTION_BUNDLE_IDENTIFIER,
};

#[test]
fn production_profile_preserves_storage_and_presentation_byte_for_byte() {
    let profile = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some(PRODUCTION_BUNDLE_IDENTIFIER.into()),
        ..BundleProfileMetadata::default()
    })
    .expect("production metadata should resolve");

    let storage = profile
        .storage(
            PathBuf::from("/Users/example").as_path(),
            StorageOverrides::default(),
        )
        .expect("production storage should resolve");
    assert_eq!(
        storage.preferences_path,
        PathBuf::from("/Users/example/Library/Application Support/Statlet/preferences.json")
    );
    assert_eq!(
        storage.history_path,
        PathBuf::from("/Users/example/Library/Application Support/Statlet/history.json")
    );
    assert_eq!(
        storage.icon_assets_directory,
        PathBuf::from("/Users/example/Library/Application Support/Statlet/indicator-icons")
    );

    let presentation = profile.presentation();
    assert_eq!(
        presentation.window_title("Histórico do Statlet"),
        "Histórico do Statlet"
    );
    assert_eq!(
        presentation.status_metadata("CPU 42%, RAM 68%"),
        "CPU 42%, RAM 68%"
    );
    assert_eq!(presentation.dev_marker(), None);
    assert_eq!(presentation.menu_identity(), None);
    assert_eq!(
        presentation.notification_title("O disco continua acima do limite"),
        "O disco continua acima do limite"
    );
    assert_eq!(
        presentation.notification_request_id("statlet.disk.123"),
        "statlet.disk.123"
    );
}

#[test]
fn development_profiles_isolate_storage_and_identify_every_presentation() {
    let profile_a = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-a-0123456789ab".into()),
        dev_display_name: Some("Task A".into()),
        dev_short_marker: Some("0123".into()),
    })
    .expect("complete Dev A metadata should resolve");
    let profile_b = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-b-abcdef012345".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-b-abcdef012345".into()),
        dev_display_name: Some("Task B".into()),
        dev_short_marker: Some("ABCD".into()),
    })
    .expect("complete Dev B metadata should resolve");

    let storage_a = profile_a
        .storage(
            PathBuf::from("/Users/example").as_path(),
            StorageOverrides::default(),
        )
        .expect("Dev A storage should resolve");
    let storage_b = profile_b
        .storage(
            PathBuf::from("/Users/example").as_path(),
            StorageOverrides::default(),
        )
        .expect("Dev B storage should resolve");
    assert_eq!(
        storage_a.preferences_path,
        PathBuf::from(
            "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/preferences.json"
        )
    );
    assert_eq!(
        storage_a.history_path,
        PathBuf::from(
            "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/history.json"
        )
    );
    assert_eq!(
        storage_a.icon_assets_directory,
        PathBuf::from(
            "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/indicator-icons"
        )
    );
    assert_ne!(storage_a, storage_b);

    let presentation = profile_a.presentation();
    assert_eq!(
        presentation.window_title("Histórico do Statlet"),
        "Histórico do Statlet — Dev 0123: Task A"
    );
    assert_eq!(
        presentation.status_metadata("CPU 42%, RAM 68%"),
        "Statlet Dev — Task A (task-a-0123456789ab): CPU 42%, RAM 68%"
    );
    assert_eq!(presentation.dev_marker(), Some("D:0123"));
    assert_eq!(
        presentation.menu_identity().as_deref(),
        Some("Statlet Dev — Task A · D:0123 · task-a-0123456789ab")
    );
    assert_eq!(
        presentation.notification_title("O disco continua acima do limite"),
        "O disco continua acima do limite — Dev 0123: Task A"
    );
    assert_eq!(
        presentation.notification_request_id("statlet.disk.123"),
        "statlet.disk.123.dev.task-a-0123456789ab"
    );
}

#[test]
fn development_profile_rejects_storage_overrides_into_production_namespace() {
    let profile = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-a-0123456789ab".into()),
        dev_display_name: Some("Task A".into()),
        dev_short_marker: Some("0123".into()),
    })
    .expect("complete Dev metadata should resolve");

    let result = profile.storage(
        PathBuf::from("/Users/example").as_path(),
        StorageOverrides {
            preferences_path: Some(PathBuf::from(
                "/Users/example/Library/Application Support/Statlet/preferences.json",
            )),
            icon_assets_directory: None,
        },
    );

    assert!(result.is_err());
}

#[test]
fn storage_rejects_relative_overrides_for_every_runtime_profile() {
    let production = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some(PRODUCTION_BUNDLE_IDENTIFIER.into()),
        ..BundleProfileMetadata::default()
    })
    .unwrap();
    let development = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-a-0123456789ab".into()),
        dev_display_name: Some("Task A".into()),
        dev_short_marker: Some("0123".into()),
    })
    .unwrap();

    for profile in [production, development] {
        let result = profile.storage(
            PathBuf::from("/Users/example").as_path(),
            StorageOverrides {
                preferences_path: Some(PathBuf::from("relative/preferences.json")),
                icon_assets_directory: None,
            },
        );
        assert!(result.is_err());
    }
}

#[test]
fn development_storage_rejects_lexical_escape_from_its_instance_root() {
    let profile = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-a-0123456789ab".into()),
        dev_display_name: Some("Task A".into()),
        dev_short_marker: Some("0123".into()),
    })
    .unwrap();
    let dev_root =
        PathBuf::from("/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab");

    let result = profile.storage(
        PathBuf::from("/Users/example").as_path(),
        StorageOverrides {
            preferences_path: Some(dev_root.join("../../preferences.json")),
            icon_assets_directory: None,
        },
    );

    assert!(result.is_err());
}

#[test]
fn development_storage_preserves_absolute_overrides_inside_its_instance_root() {
    let profile = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-a-0123456789ab".into()),
        dev_display_name: Some("Task A".into()),
        dev_short_marker: Some("0123".into()),
    })
    .unwrap();
    let preferences = PathBuf::from(
        "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/custom.json",
    );

    let storage = profile
        .storage(
            PathBuf::from("/Users/example").as_path(),
            StorageOverrides {
                preferences_path: Some(preferences.clone()),
                icon_assets_directory: None,
            },
        )
        .unwrap();

    assert_eq!(storage.preferences_path, preferences);
}

#[test]
fn runtime_profile_manifest_validation_fails_closed() {
    let invalid = [
        BundleProfileMetadata {
            bundle_identifier: Some(PRODUCTION_BUNDLE_IDENTIFIER.into()),
            dev_instance_id: Some("task-a-0123456789ab".into()),
            ..BundleProfileMetadata::default()
        },
        BundleProfileMetadata {
            bundle_identifier: Some(
                "io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into(),
            ),
            runtime_profile: Some("development".into()),
            dev_instance_id: Some("task-a-0123456789ab".into()),
            dev_display_name: Some("Task A".into()),
            dev_short_marker: None,
        },
        BundleProfileMetadata {
            bundle_identifier: Some(
                "io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into(),
            ),
            runtime_profile: Some("development".into()),
            dev_instance_id: Some("../task-a-0123456789ab".into()),
            dev_display_name: Some("Task A".into()),
            dev_short_marker: Some("0123".into()),
        },
        BundleProfileMetadata {
            bundle_identifier: Some(PRODUCTION_BUNDLE_IDENTIFIER.into()),
            runtime_profile: Some("preview".into()),
            ..BundleProfileMetadata::default()
        },
    ];

    for metadata in invalid {
        assert!(RuntimeProfile::resolve(metadata).is_err());
    }
}

#[test]
fn explicit_storage_overrides_are_preserved_outside_the_production_namespace() {
    let profile = RuntimeProfile::resolve(BundleProfileMetadata {
        bundle_identifier: Some("io.github.hayashirafael.Statlet.dev.task-a-0123456789ab".into()),
        runtime_profile: Some("development".into()),
        dev_instance_id: Some("task-a-0123456789ab".into()),
        dev_display_name: Some("Task A".into()),
        dev_short_marker: Some("0123".into()),
    })
    .unwrap();
    let storage = profile
        .storage(
            PathBuf::from("/Users/example").as_path(),
            StorageOverrides {
                preferences_path: Some(PathBuf::from("/tmp/statlet-a/preferences.json")),
                icon_assets_directory: Some(PathBuf::from("/tmp/statlet-a/icons")),
            },
        )
        .unwrap();

    assert_eq!(
        storage.preferences_path,
        PathBuf::from("/tmp/statlet-a/preferences.json")
    );
    assert_eq!(
        storage.icon_assets_directory,
        PathBuf::from("/tmp/statlet-a/icons")
    );
    assert_eq!(
        storage.history_path,
        PathBuf::from(
            "/Users/example/Library/Application Support/Statlet/Dev/task-a-0123456789ab/history.json"
        )
    );
}
