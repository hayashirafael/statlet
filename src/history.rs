use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const CURRENT_VERSION: u8 = 1;
pub const MAX_HISTORY_RECORDS: usize = 30;
static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryEventKind {
    DiskPressureStarted,
    DiskPressureRecovered,
    MoleMissing,
    MoleIncompatible,
    MoleUnavailable,
    MonitoringFailed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub timestamp_unix_seconds: u64,
    pub kind: HistoryEventKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct History {
    records: Vec<HistoryRecord>,
}

impl History {
    pub fn records(&self) -> &[HistoryRecord] {
        &self.records
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_current_user() -> io::Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unavailable"))?;
        Ok(Self::new(
            home.join("Library/Application Support/Statlet/history.json"),
        ))
    }

    pub fn load(&self) -> History {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredHistory>(&bytes).ok())
            .and_then(StoredHistory::into_history)
            .unwrap_or_default()
    }

    pub fn record(&self, kind: HistoryEventKind, at: SystemTime) -> io::Result<History> {
        let timestamp_unix_seconds = at
            .duration_since(UNIX_EPOCH)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "timestamp predates epoch"))?
            .as_secs();
        let mut history = self.load();
        history.records.insert(
            0,
            HistoryRecord {
                timestamp_unix_seconds,
                kind,
            },
        );
        history.records.truncate(MAX_HISTORY_RECORDS);
        self.save(&history)?;
        Ok(history)
    }

    pub fn clear(&self) -> io::Result<History> {
        let history = History::default();
        self.save(&history)?;
        Ok(history)
    }

    fn save(&self, history: &History) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "history path has no parent")
        })?;
        fs::create_dir_all(parent)?;
        let (temporary_path, mut file) = create_temporary_file(parent, &self.path)?;
        let result = (|| {
            serde_json::to_writer_pretty(&mut file, &StoredHistory::from(history))
                .map_err(io::Error::other)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&temporary_path, &self.path)?;
            File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary_path);
        }
        result
    }
}

fn create_temporary_file(parent: &Path, destination: &Path) -> io::Result<(PathBuf, File)> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("history.json");
    for _ in 0..16 {
        let id = NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{file_name}.{}.{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary history file",
    ))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredHistory {
    version: u8,
    records: Vec<HistoryRecord>,
}

impl From<&History> for StoredHistory {
    fn from(history: &History) -> Self {
        Self {
            version: CURRENT_VERSION,
            records: history.records.clone(),
        }
    }
}

impl StoredHistory {
    fn into_history(mut self) -> Option<History> {
        if self.version != CURRENT_VERSION {
            return None;
        }
        self.records
            .sort_by_key(|record| std::cmp::Reverse(record.timestamp_unix_seconds));
        self.records.truncate(MAX_HISTORY_RECORDS);
        Some(History {
            records: self.records,
        })
    }
}
