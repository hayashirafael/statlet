use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::{Preferences, WarningThreshold};

const CURRENT_VERSION: u8 = 1;

#[derive(Clone, Debug)]
pub struct PreferencesStore {
    path: PathBuf,
}

impl PreferencesStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_current_user() -> io::Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unavailable"))?;
        Ok(Self::new(home.join(
            "Library/Application Support/Statlet/preferences.json",
        )))
    }

    pub fn load(&self) -> Preferences {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredPreferences>(&bytes).ok())
            .and_then(StoredPreferences::into_preferences)
            .unwrap_or_default()
    }

    pub fn save(&self, preferences: Preferences) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "preferences path has no parent",
            )
        })?;
        fs::create_dir_all(parent)?;

        let temporary_path = temporary_path(&self.path);
        let result = self.write_and_replace(parent, &temporary_path, preferences);
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    fn write_and_replace(
        &self,
        parent: &Path,
        temporary_path: &Path,
        preferences: Preferences,
    ) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary_path)?;
        let stored = StoredPreferences::from(preferences);
        serde_json::to_writer_pretty(&mut file, &stored).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(temporary_path, &self.path)?;
        File::open(parent)?.sync_all()
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("preferences.json");
    destination.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreferences {
    version: u8,
    mole_integration_enabled: bool,
    warning_threshold: u8,
}

impl From<Preferences> for StoredPreferences {
    fn from(preferences: Preferences) -> Self {
        Self {
            version: CURRENT_VERSION,
            mole_integration_enabled: preferences.mole_integration_enabled,
            warning_threshold: preferences.warning_threshold.get(),
        }
    }
}

impl StoredPreferences {
    fn into_preferences(self) -> Option<Preferences> {
        if self.version != CURRENT_VERSION {
            return None;
        }

        Some(Preferences {
            mole_integration_enabled: self.mole_integration_enabled,
            warning_threshold: WarningThreshold::try_from(self.warning_threshold).ok()?,
        })
    }
}
