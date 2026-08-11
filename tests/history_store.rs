use std::fs;
use std::time::{Duration, UNIX_EPOCH};

use statlet::history::{HistoryEventKind, HistoryStore};
use tempfile::tempdir;

#[test]
fn records_are_newest_first_and_truncated_to_thirty() {
    let directory = tempdir().unwrap();
    let store = HistoryStore::new(directory.path().join("history.json"));

    for second in 0..35 {
        store
            .record(
                HistoryEventKind::DiskPressureStarted,
                UNIX_EPOCH + Duration::from_secs(second),
            )
            .unwrap();
    }

    let history = store.load();
    assert_eq!(history.records().len(), 30);
    assert_eq!(
        history.records().first().unwrap().timestamp_unix_seconds,
        34
    );
    assert_eq!(history.records().last().unwrap().timestamp_unix_seconds, 5);
}

#[test]
fn clear_atomically_persists_an_empty_versioned_history() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("history.json");
    let store = HistoryStore::new(path.clone());
    store
        .record(HistoryEventKind::MonitoringFailed, UNIX_EPOCH)
        .unwrap();

    let history = store.clear().unwrap();

    assert!(history.is_empty());
    assert!(store.load().is_empty());
    let json = fs::read_to_string(path).unwrap();
    assert!(json.contains("\"version\": 1"));
    assert!(json.contains("\"records\": []"));
}

#[test]
fn corrupt_or_unknown_data_falls_back_and_is_replaced_on_next_record() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("history.json");
    let store = HistoryStore::new(path.clone());
    fs::write(&path, b"not json").unwrap();
    assert!(store.load().is_empty());

    store
        .record(HistoryEventKind::MoleMissing, UNIX_EPOCH)
        .unwrap();
    assert_eq!(
        store.load().records()[0].kind,
        HistoryEventKind::MoleMissing
    );

    fs::write(&path, br#"{"version":99,"records":[]}"#).unwrap();
    assert!(store.load().is_empty());
}

#[test]
fn persisted_records_have_only_timestamp_and_concise_state() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("history.json");
    let store = HistoryStore::new(path.clone());
    for kind in [
        HistoryEventKind::DiskPressureStarted,
        HistoryEventKind::DiskPressureRecovered,
        HistoryEventKind::MoleMissing,
        HistoryEventKind::MoleIncompatible,
        HistoryEventKind::MoleUnavailable,
        HistoryEventKind::MonitoringFailed,
    ] {
        store.record(kind, UNIX_EPOCH).unwrap();
    }

    let json = fs::read_to_string(path).unwrap();

    assert!(!json.to_ascii_lowercase().contains("filename"));
    assert!(!json.to_ascii_lowercase().contains("filepath"));
    assert!(!json.contains("/Users/"));
    assert!(json.contains("timestampUnixSeconds"));
}
