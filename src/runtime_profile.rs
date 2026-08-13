use std::fmt;
use std::path::{Component, Path, PathBuf};

pub const PRODUCTION_BUNDLE_IDENTIFIER: &str = "io.github.hayashirafael.Statlet";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BundleProfileMetadata {
    pub bundle_identifier: Option<String>,
    pub runtime_profile: Option<String>,
    pub dev_instance_id: Option<String>,
    pub dev_display_name: Option<String>,
    pub dev_short_marker: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeProfile {
    Production,
    Development(DevIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevIdentity {
    instance_id: String,
    display_name: String,
    short_marker: String,
    status_marker: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StorageOverrides {
    pub preferences_path: Option<PathBuf>,
    pub icon_assets_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStorage {
    pub preferences_path: PathBuf,
    pub history_path: PathBuf,
    pub icon_assets_directory: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimePresentation {
    development: Option<DevIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeProfileError(String);

impl fmt::Display for RuntimeProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeProfileError {}

impl RuntimeProfile {
    pub fn resolve(metadata: BundleProfileMetadata) -> Result<Self, RuntimeProfileError> {
        if metadata.bundle_identifier.as_deref() == Some(PRODUCTION_BUNDLE_IDENTIFIER)
            && metadata.runtime_profile.is_none()
            && metadata.dev_instance_id.is_none()
            && metadata.dev_display_name.is_none()
            && metadata.dev_short_marker.is_none()
        {
            Ok(Self::Production)
        } else if metadata.runtime_profile.as_deref() == Some("development") {
            let instance_id = metadata.dev_instance_id.ok_or_else(invalid_metadata)?;
            let display_name = metadata.dev_display_name.ok_or_else(invalid_metadata)?;
            let short_marker = metadata.dev_short_marker.ok_or_else(invalid_metadata)?;
            let expected_bundle_identifier =
                format!("{PRODUCTION_BUNDLE_IDENTIFIER}.dev.{instance_id}");
            if metadata.bundle_identifier.as_deref() != Some(&expected_bundle_identifier)
                || !valid_instance_id(&instance_id)
                || !valid_display_name(&display_name)
                || marker_for_instance(&instance_id).as_deref() != Some(&short_marker)
            {
                return Err(invalid_metadata());
            }
            let status_marker = format!("D:{short_marker}");
            Ok(Self::Development(DevIdentity {
                instance_id,
                display_name,
                short_marker,
                status_marker,
            }))
        } else {
            Err(invalid_metadata())
        }
    }

    pub fn storage(
        &self,
        home: &Path,
        overrides: StorageOverrides,
    ) -> Result<RuntimeStorage, RuntimeProfileError> {
        let production_root = home.join("Library/Application Support/Statlet");
        let root = match self {
            Self::Production => production_root.clone(),
            Self::Development(identity) => production_root.join("Dev").join(&identity.instance_id),
        };
        let preferences_path = overrides
            .preferences_path
            .map(|path| self.validate_storage_override(&path, &production_root, &root))
            .transpose()?;
        let icon_assets_directory = overrides
            .icon_assets_directory
            .map(|path| self.validate_storage_override(&path, &production_root, &root))
            .transpose()?;
        Ok(RuntimeStorage {
            preferences_path: preferences_path.unwrap_or_else(|| root.join("preferences.json")),
            history_path: root.join("history.json"),
            icon_assets_directory: icon_assets_directory
                .unwrap_or_else(|| root.join("indicator-icons")),
        })
    }

    fn validate_storage_override(
        &self,
        path: &Path,
        production_root: &Path,
        profile_root: &Path,
    ) -> Result<PathBuf, RuntimeProfileError> {
        let normalized = lexically_normalize_absolute(path)?;
        if matches!(self, Self::Development(_))
            && normalized.starts_with(production_root)
            && !normalized.starts_with(profile_root)
        {
            return Err(RuntimeProfileError(
                "development storage cannot use the production namespace".into(),
            ));
        }
        Ok(normalized)
    }

    pub fn presentation(&self) -> RuntimePresentation {
        RuntimePresentation {
            development: match self {
                Self::Production => None,
                Self::Development(identity) => Some(identity.clone()),
            },
        }
    }
}

impl RuntimePresentation {
    pub fn window_title(&self, production_title: &str) -> String {
        self.development.as_ref().map_or_else(
            || production_title.to_owned(),
            |identity| {
                format!(
                    "{production_title} — Dev {}: {}",
                    identity.short_marker, identity.display_name
                )
            },
        )
    }

    pub fn status_metadata(&self, production_label: &str) -> String {
        self.development.as_ref().map_or_else(
            || production_label.to_owned(),
            |identity| {
                format!(
                    "Statlet Dev — {} ({}): {production_label}",
                    identity.display_name, identity.instance_id
                )
            },
        )
    }

    pub fn dev_marker(&self) -> Option<&str> {
        self.development
            .as_ref()
            .map(|identity| identity.status_marker.as_str())
    }

    pub fn menu_identity(&self) -> Option<String> {
        self.development.as_ref().map(|identity| {
            format!(
                "Statlet Dev — {} · {} · {}",
                identity.display_name, identity.status_marker, identity.instance_id
            )
        })
    }

    pub fn notification_title(&self, production_title: &str) -> String {
        self.development.as_ref().map_or_else(
            || production_title.to_owned(),
            |identity| {
                format!(
                    "{production_title} — Dev {}: {}",
                    identity.short_marker, identity.display_name
                )
            },
        )
    }

    pub fn notification_request_id(&self, production_id: &str) -> String {
        self.development.as_ref().map_or_else(
            || production_id.to_owned(),
            |identity| format!("{production_id}.dev.{}", identity.instance_id),
        )
    }
}

fn invalid_metadata() -> RuntimeProfileError {
    RuntimeProfileError("invalid runtime profile metadata".into())
}

fn lexically_normalize_absolute(path: &Path) -> Result<PathBuf, RuntimeProfileError> {
    if !path.is_absolute() {
        return Err(RuntimeProfileError(
            "storage overrides must be absolute paths".into(),
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(RuntimeProfileError(
                        "storage override escapes the filesystem root".into(),
                    ));
                }
            }
        }
    }
    Ok(normalized)
}

fn valid_instance_id(instance_id: &str) -> bool {
    let Some((slug, digest)) = instance_id.rsplit_once('-') else {
        return false;
    };
    !slug.is_empty()
        && slug.len() <= 24
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && digest.len() == 12
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn marker_for_instance(instance_id: &str) -> Option<String> {
    let (_, digest) = instance_id.rsplit_once('-')?;
    Some(digest.get(..4)?.to_ascii_uppercase())
}

fn valid_display_name(display_name: &str) -> bool {
    !display_name.trim().is_empty()
        && display_name.chars().count() <= 80
        && !display_name.chars().any(char::is_control)
}
