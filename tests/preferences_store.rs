use std::fs;

use statlet::core::{Preferences, WarningThreshold};
use statlet::preferences::PreferencesStore;
use tempfile::tempdir;

#[test]
fn missing_preferences_load_safe_defaults() {
    let directory = tempdir().unwrap();
    let store = PreferencesStore::new(directory.path().join("preferences.json"));

    assert_eq!(store.load(), Preferences::default());
}

#[test]
fn corrupt_or_unsupported_preferences_load_safe_defaults() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());

    fs::write(&path, "not json").unwrap();
    assert_eq!(store.load(), Preferences::default());

    fs::write(
        &path,
        r#"{"version":2,"moleIntegrationEnabled":true,"warningThreshold":95}"#,
    )
    .unwrap();
    assert_eq!(store.load(), Preferences::default());

    fs::write(
        &path,
        r#"{"version":1,"moleIntegrationEnabled":true,"warningThreshold":91}"#,
    )
    .unwrap();
    assert_eq!(store.load(), Preferences::default());
}

#[test]
fn valid_versioned_preferences_round_trip_with_atomic_replacement() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("nested/preferences.json");
    let store = PreferencesStore::new(path.clone());
    let first = Preferences {
        mole_integration_enabled: true,
        warning_threshold: WarningThreshold::try_from(80).unwrap(),
    };
    let second = Preferences {
        mole_integration_enabled: false,
        warning_threshold: WarningThreshold::try_from(95).unwrap(),
    };

    store.save(first).unwrap();
    assert_eq!(store.load(), first);
    store.save(second).unwrap();
    assert_eq!(store.load(), second);

    let json = fs::read_to_string(&path).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({
            "version": 1,
            "moleIntegrationEnabled": false,
            "warningThreshold": 95
        })
    );
    assert_eq!(
        fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1,
        "atomic save must not leave temporary files behind"
    );
}
