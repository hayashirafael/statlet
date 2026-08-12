use std::fs;

use statlet::core::{Preferences, WarningThreshold};
use statlet::indicator_preferences::{
    FontFamilyPreference, IndicatorPreferences, MetricsRefreshInterval,
};
use statlet::preferences::PreferencesStore;
use tempfile::tempdir;

#[test]
fn missing_preferences_load_safe_defaults() {
    let directory = tempdir().unwrap();
    let store = PreferencesStore::new(directory.path().join("preferences.json"));

    assert_eq!(store.load(), Preferences::default());
}

#[test]
fn version_one_migrates_disk_values_and_defaults_the_indicator() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());

    fs::write(
        &path,
        r#"{"version":1,"moleIntegrationEnabled":true,"warningThreshold":95}"#,
    )
    .unwrap();

    let loaded = store.load();

    assert!(loaded.mole_integration_enabled);
    assert_eq!(loaded.warning_threshold.get(), 95);
    assert_eq!(loaded.indicator, IndicatorPreferences::default());
}

#[test]
fn version_two_round_trip_preserves_nested_indicator_preferences() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let mut expected = Preferences::default();
    expected.indicator.refresh_interval = MetricsRefreshInterval::try_from(17).unwrap();
    expected.indicator.typography.family = FontFamilyPreference::named("Avenir Next").unwrap();

    store.save(expected.clone()).unwrap();

    assert_eq!(store.load(), expected);
    let saved =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert_eq!(
        saved["indicator"]["typography"]["family"],
        serde_json::json!({ "named": "Avenir Next" })
    );
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
fn invalid_nested_version_two_values_load_safe_defaults() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let valid = serde_json::json!({
        "version": 2,
        "moleIntegrationEnabled": true,
        "warningThreshold": 95,
        "indicator": {
            "cpuColor": {
                "mode": "dynamic",
                "fixed": {
                    "shared": "#34C759",
                    "useAppearanceVariants": false,
                    "variants": null
                }
            },
            "ramColor": {
                "mode": "dynamic",
                "fixed": {
                    "shared": "#0A84FF",
                    "useAppearanceVariants": false,
                    "variants": null
                }
            },
            "labels": {
                "visible": true,
                "colorMode": "neutral",
                "fixed": {
                    "shared": "#8E8E93",
                    "useAppearanceVariants": false,
                    "variants": null
                }
            },
            "typography": {
                "family": "systemMonospaced",
                "size": 12,
                "weight": "medium"
            },
            "refreshInterval": 2
        }
    });
    let invalid_values = [
        (
            "color with alpha",
            "/indicator/cpuColor/fixed/shared",
            serde_json::json!("#34C759FF"),
        ),
        (
            "font size below minimum",
            "/indicator/typography/size",
            serde_json::json!(8),
        ),
        (
            "font size above maximum",
            "/indicator/typography/size",
            serde_json::json!(15),
        ),
        (
            "zero refresh interval",
            "/indicator/refreshInterval",
            serde_json::json!(0),
        ),
        (
            "refresh interval above maximum",
            "/indicator/refreshInterval",
            serde_json::json!(61),
        ),
        (
            "blank named font",
            "/indicator/typography/family",
            serde_json::json!({ "named": "   " }),
        ),
        ("unsupported version", "/version", serde_json::json!(3)),
    ];

    for (case, pointer, invalid_value) in invalid_values {
        let mut payload = valid.clone();
        *payload.pointer_mut(pointer).unwrap() = invalid_value;
        fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(store.load(), Preferences::default(), "{case}");
    }

    let mut missing_indicator = valid;
    missing_indicator
        .as_object_mut()
        .unwrap()
        .remove("indicator");
    fs::write(&path, serde_json::to_vec(&missing_indicator).unwrap()).unwrap();
    assert_eq!(store.load(), Preferences::default(), "missing indicator");
}

#[test]
fn valid_versioned_preferences_round_trip_with_atomic_replacement() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("nested/preferences.json");
    let store = PreferencesStore::new(path.clone());
    let first = Preferences {
        mole_integration_enabled: true,
        warning_threshold: WarningThreshold::try_from(80).unwrap(),
        ..Preferences::default()
    };
    let second = Preferences {
        mole_integration_enabled: false,
        warning_threshold: WarningThreshold::try_from(95).unwrap(),
        ..Preferences::default()
    };

    store.save(first.clone()).unwrap();
    assert_eq!(store.load(), first);
    store.save(second.clone()).unwrap();
    assert_eq!(store.load(), second);

    let json = fs::read_to_string(&path).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({
            "version": 2,
            "moleIntegrationEnabled": false,
            "warningThreshold": 95,
            "indicator": {
                "cpuColor": {
                    "mode": "dynamic",
                    "fixed": {
                        "shared": "#34C759",
                        "useAppearanceVariants": false,
                        "variants": null
                    }
                },
                "ramColor": {
                    "mode": "dynamic",
                    "fixed": {
                        "shared": "#0A84FF",
                        "useAppearanceVariants": false,
                        "variants": null
                    }
                },
                "labels": {
                    "visible": true,
                    "colorMode": "neutral",
                    "fixed": {
                        "shared": "#8E8E93",
                        "useAppearanceVariants": false,
                        "variants": null
                    }
                },
                "typography": {
                    "family": "systemMonospaced",
                    "size": 12,
                    "weight": "medium"
                },
                "refreshInterval": 2
            }
        })
    );
    assert_eq!(
        fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1,
        "atomic save must not leave temporary files behind"
    );
}
