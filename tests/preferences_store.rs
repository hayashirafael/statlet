use std::fs;

use statlet::core::{Preferences, WarningThreshold};
use statlet::indicator_preferences::{
    FontFamilyPreference, IndicatorLabel, IndicatorPreferences, LabelSpacing,
    MetricsRefreshInterval, SystemSymbolSize,
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
fn menu_bar_visibility_round_trips_in_the_current_document() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let expected = Preferences {
        show_in_menu_bar: false,
        ..Preferences::default()
    };

    store.save(expected.clone()).unwrap();

    assert_eq!(store.load(), expected);
    let saved =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved["version"], 4);
    assert_eq!(saved["showInMenuBar"], false);
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

    assert!(loaded.show_in_menu_bar);
    assert!(loaded.mole_integration_enabled);
    assert_eq!(loaded.warning_threshold.get(), 95);
    assert_eq!(loaded.indicator, IndicatorPreferences::default());
    assert_eq!(loaded.indicator.labels.cpu.as_str(), "C");
    assert_eq!(loaded.indicator.labels.ram.as_str(), "R");
    assert_eq!(loaded.indicator.labels.spacing.level(), 10);
}

#[test]
fn version_three_defaults_visibility_without_losing_existing_configuration() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let mut expected = Preferences {
        show_in_menu_bar: true,
        mole_integration_enabled: true,
        warning_threshold: WarningThreshold::try_from(85).unwrap(),
        ..Preferences::default()
    };
    expected.indicator.refresh_interval = MetricsRefreshInterval::try_from(17).unwrap();
    expected.indicator.labels.cpu = IndicatorLabel::new("CPU uso").unwrap();
    expected.indicator.labels.spacing = LabelSpacing::try_from(3).unwrap();
    let persisted = Preferences {
        show_in_menu_bar: false,
        ..expected.clone()
    };
    store.save(persisted).unwrap();
    let mut payload =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap();
    payload["version"] = serde_json::json!(3);
    payload.as_object_mut().unwrap().remove("showInMenuBar");
    fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();

    assert_eq!(store.load(), expected);
}

#[test]
fn version_four_round_trip_preserves_nested_indicator_preferences() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let mut expected = Preferences::default();
    expected.indicator.refresh_interval = MetricsRefreshInterval::try_from(17).unwrap();
    expected.indicator.typography.family = FontFamilyPreference::named("Avenir Next").unwrap();
    expected.indicator.identifiers.system_symbol_size = SystemSymbolSize::try_from(14).unwrap();

    store.save(expected.clone()).unwrap();

    assert_eq!(store.load(), expected);
    let saved =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved["version"], 4);
    assert_eq!(saved["showInMenuBar"], true);
    assert_eq!(saved["indicator"]["identifiers"]["systemSymbolSize"], 14);
    assert_eq!(
        saved["indicator"]["typography"]["family"],
        serde_json::json!({ "named": "Avenir Next" })
    );
}

#[test]
fn version_four_without_system_symbol_size_keeps_the_compatible_twelve_point_default() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    store.save(Preferences::default()).unwrap();
    let mut payload =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap();
    payload["indicator"]["identifiers"]
        .as_object_mut()
        .unwrap()
        .remove("systemSymbolSize");
    fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();

    assert_eq!(
        store
            .load()
            .indicator
            .identifiers
            .system_symbol_size
            .points(),
        12
    );
}

#[test]
fn version_four_rejects_system_symbol_size_outside_the_safe_range() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    store.save(Preferences::default()).unwrap();

    for invalid in [7, 15] {
        let mut payload =
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap();
        payload["indicator"]["identifiers"]["systemSymbolSize"] = serde_json::json!(invalid);
        fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(store.load(), Preferences::default(), "invalid {invalid}");
        store.save(Preferences::default()).unwrap();
    }
}

#[test]
fn version_four_round_trip_preserves_custom_labels_and_spacing() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());
    let mut expected = Preferences::default();
    expected.indicator.labels.cpu = IndicatorLabel::new("CPU uso").unwrap();
    expected.indicator.labels.ram = IndicatorLabel::new("Memória").unwrap();
    expected.indicator.labels.spacing = LabelSpacing::try_from(3).unwrap();

    store.save(expected.clone()).unwrap();

    assert_eq!(store.load(), expected);
    let saved =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved["indicator"]["labels"]["cpu"], "CPU uso");
    assert_eq!(saved["indicator"]["labels"]["ram"], "Memória");
    assert_eq!(saved["indicator"]["labels"]["spacing"], 3);
}

#[test]
fn version_two_spacing_migrates_legacy_spaces_to_decimal_levels() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());

    let mut expected = Preferences {
        mole_integration_enabled: true,
        warning_threshold: WarningThreshold::try_from(85).unwrap(),
        ..Preferences::default()
    };
    expected.indicator.labels.cpu = IndicatorLabel::new("CPU uso").unwrap();
    expected.indicator.refresh_interval = MetricsRefreshInterval::try_from(17).unwrap();
    store.save(expected.clone()).unwrap();
    let stored =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap();

    for (legacy, expected_spacing) in [(0, 0), (1, 10), (4, 10)] {
        let mut v2 = stored.clone();
        v2["version"] = serde_json::json!(2);
        v2.as_object_mut().unwrap().remove("showInMenuBar");
        v2["indicator"]["labels"]["spacing"] = serde_json::json!(legacy);
        fs::write(&path, serde_json::to_vec(&v2).unwrap()).unwrap();

        let loaded = store.load();
        assert!(loaded.show_in_menu_bar);
        let mut expected = expected.clone();
        expected.indicator.labels.spacing = LabelSpacing::try_from(expected_spacing).unwrap();
        assert_eq!(loaded, expected, "legacy spacing {legacy}");
    }
}

#[test]
fn version_two_spacing_above_the_legacy_range_loads_safe_defaults() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let store = PreferencesStore::new(path.clone());

    store.save(Preferences::default()).unwrap();
    let mut stored =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap();
    stored["version"] = serde_json::json!(2);
    stored.as_object_mut().unwrap().remove("showInMenuBar");
    stored["moleIntegrationEnabled"] = serde_json::json!(true);
    stored["indicator"]["labels"]["spacing"] = serde_json::json!(5);
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();

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
        ("unsupported version", "/version", serde_json::json!(5)),
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
            "version": 4,
            "showInMenuBar": true,
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
                "identifiers": {
                    "systemSymbolSize": 12,
                    "cpu": {
                        "mode": "text",
                        "systemSymbol": "cpu",
                        "png": null
                    },
                    "ram": {
                        "mode": "text",
                        "systemSymbol": "memorychip",
                        "png": null
                    }
                },
                "labels": {
                    "visible": true,
                    "colorMode": "neutral",
                    "fixed": {
                        "shared": "#8E8E93",
                        "useAppearanceVariants": false,
                        "variants": null
                    },
                    "cpu": "C",
                    "ram": "R",
                    "spacing": 10
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
