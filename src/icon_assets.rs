use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader};

use crate::indicator_preferences::{MetricKind, PngIconMetadata};

pub const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_DIMENSION: u32 = 8_192;
const MAX_SOURCE_PIXELS: u64 = 16_777_216;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TEMPORARY_PATH_ATTEMPTS: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PngImportErrorKind {
    SourceTooLarge,
    InvalidPng,
    PixelLimitExceeded,
    InvalidMetadata,
    FileSystem,
}

#[derive(Debug)]
pub struct PngImportError {
    kind: PngImportErrorKind,
    message: String,
}

impl PngImportError {
    fn new(kind: PngImportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub const fn kind(&self) -> PngImportErrorKind {
        self.kind
    }

    pub fn user_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PngImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PngImportError {}

#[derive(Debug)]
pub struct NormalizedPng {
    bytes: Vec<u8>,
    metadata: PngIconMetadata,
}

impl NormalizedPng {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn metadata(&self) -> &PngIconMetadata {
        &self.metadata
    }
}

pub fn normalize_png(
    source_name: impl AsRef<str>,
    source: &[u8],
) -> Result<NormalizedPng, PngImportError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(PngImportError::new(
            PngImportErrorKind::SourceTooLarge,
            "O PNG excede o limite de 4 MB.",
        ));
    }
    let (width, height) = png_dimensions(source)?;
    if width > MAX_SOURCE_DIMENSION
        || height > MAX_SOURCE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS
    {
        return Err(PngImportError::new(
            PngImportErrorKind::PixelLimitExceeded,
            "O PNG tem dimensões grandes demais para o indicador.",
        ));
    }

    let mut reader = ImageReader::with_format(Cursor::new(source), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| invalid_png())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| invalid_png())?;
    image.apply_orientation(orientation);
    let (width, height) = image.dimensions();
    if width > PngIconMetadata::MAX_DIMENSION || height > PngIconMetadata::MAX_DIMENSION {
        image = image.resize(
            PngIconMetadata::MAX_DIMENSION,
            PngIconMetadata::MAX_DIMENSION,
            FilterType::Lanczos3,
        );
    }

    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|_| invalid_png())?;
    let bytes = encoded.into_inner();
    let (width, height) = image.dimensions();
    let metadata = PngIconMetadata::with_content_fingerprint(
        source_name.as_ref(),
        width,
        height,
        bytes.len() as u64,
        fnv1a(&bytes),
    )
    .map_err(|_| {
        PngImportError::new(
            PngImportErrorKind::InvalidMetadata,
            "O arquivo precisa ter um nome PNG válido.",
        )
    })?;
    Ok(NormalizedPng { bytes, metadata })
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn png_dimensions(source: &[u8]) -> Result<(u32, u32), PngImportError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if source.len() < 24 || &source[..8] != SIGNATURE || &source[12..16] != b"IHDR" {
        return Err(invalid_png());
    }
    let width = u32::from_be_bytes(source[16..20].try_into().expect("four-byte PNG width"));
    let height = u32::from_be_bytes(source[20..24].try_into().expect("four-byte PNG height"));
    if width == 0 || height == 0 {
        return Err(invalid_png());
    }
    Ok((width, height))
}

fn invalid_png() -> PngImportError {
    PngImportError::new(
        PngImportErrorKind::InvalidPng,
        "Não foi possível abrir este arquivo como PNG.",
    )
}

#[derive(Clone, Debug)]
pub struct IconAssetStore {
    directory: PathBuf,
    temp_sequence: Arc<AtomicU64>,
}

#[derive(Debug)]
pub struct PreparedPngAsset {
    metric: MetricKind,
    temporary: PathBuf,
    metadata: PngIconMetadata,
}

impl PreparedPngAsset {
    pub const fn metric(&self) -> MetricKind {
        self.metric
    }

    pub const fn metadata(&self) -> &PngIconMetadata {
        &self.metadata
    }
}

impl Drop for PreparedPngAsset {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetMutation {
    Replace,
    Remove,
}

#[derive(Debug)]
pub struct PngAssetTransaction {
    directory: PathBuf,
    destination: PathBuf,
    transaction_directory: PathBuf,
    backup: Option<PathBuf>,
    mutation: AssetMutation,
    active: bool,
}

impl PngAssetTransaction {
    pub fn commit(mut self) {
        if let Some(backup) = &self.backup {
            let _ = fs::remove_file(backup);
        }
        let _ = fs::remove_dir(&self.transaction_directory);
        let _ = File::open(&self.directory).and_then(|directory| directory.sync_all());
        self.active = false;
    }

    pub fn rollback(mut self) -> Result<(), PngImportError> {
        self.rollback_inner().map_err(file_system_error)?;
        self.active = false;
        Ok(())
    }

    fn rollback_inner(&self) -> io::Result<()> {
        if self.mutation == AssetMutation::Replace {
            match fs::remove_file(&self.destination) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(backup) = &self.backup {
            fs::rename(backup, &self.destination)?;
        }
        match fs::remove_dir(&self.transaction_directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        File::open(&self.directory)?.sync_all()
    }
}

impl Drop for PngAssetTransaction {
    fn drop(&mut self) {
        if self.active {
            let _ = self.rollback_inner();
        }
    }
}

impl IconAssetStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            temp_sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn for_current_user() -> io::Result<Self> {
        if let Some(path) = std::env::var_os("STATLET_ICON_ASSETS_DIR") {
            return Ok(Self::new(PathBuf::from(path)));
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unavailable"))?;
        Ok(Self::new(home.join(
            "Library/Application Support/Statlet/indicator-icons",
        )))
    }

    pub fn path_for(&self, metric: MetricKind) -> PathBuf {
        self.directory.join(match metric {
            MetricKind::Cpu => "cpu.png",
            MetricKind::Ram => "ram.png",
        })
    }

    pub fn import_file(
        &self,
        metric: MetricKind,
        source: &Path,
    ) -> Result<PngIconMetadata, PngImportError> {
        let metadata = fs::metadata(source).map_err(file_system_error)?;
        if metadata.len() > MAX_SOURCE_BYTES as u64 {
            return Err(PngImportError::new(
                PngImportErrorKind::SourceTooLarge,
                "O PNG excede o limite de 4 MB.",
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(source)
            .and_then(|file| {
                file.take(MAX_SOURCE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
            })
            .map_err(file_system_error)?;
        let source_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                PngImportError::new(
                    PngImportErrorKind::InvalidMetadata,
                    "O arquivo precisa ter um nome PNG válido.",
                )
            })?;
        self.import_bytes(metric, source_name, &bytes)
    }

    pub fn import_bytes(
        &self,
        metric: MetricKind,
        source_name: &str,
        bytes: &[u8],
    ) -> Result<PngIconMetadata, PngImportError> {
        let prepared = self.prepare_bytes(metric, source_name, bytes)?;
        let metadata = prepared.metadata().clone();
        self.begin_replace(prepared)?.commit();
        Ok(metadata)
    }

    pub fn prepare_file(
        &self,
        metric: MetricKind,
        source: &Path,
    ) -> Result<PreparedPngAsset, PngImportError> {
        let metadata = fs::metadata(source).map_err(file_system_error)?;
        if metadata.len() > MAX_SOURCE_BYTES as u64 {
            return Err(PngImportError::new(
                PngImportErrorKind::SourceTooLarge,
                "O PNG excede o limite de 4 MB.",
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(source)
            .and_then(|file| {
                file.take(MAX_SOURCE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
            })
            .map_err(file_system_error)?;
        let source_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                PngImportError::new(
                    PngImportErrorKind::InvalidMetadata,
                    "O arquivo precisa ter um nome PNG válido.",
                )
            })?;
        self.prepare_bytes(metric, source_name, &bytes)
    }

    pub fn prepare_bytes(
        &self,
        metric: MetricKind,
        source_name: &str,
        bytes: &[u8],
    ) -> Result<PreparedPngAsset, PngImportError> {
        let normalized = normalize_png(source_name, bytes)?;
        fs::create_dir_all(&self.directory).map_err(file_system_error)?;
        let destination = self.path_for(metric);
        let temporary = self.write_unique_temporary(&destination, normalized.bytes(), "tmp")?;
        Ok(PreparedPngAsset {
            metric,
            temporary,
            metadata: normalized.metadata,
        })
    }

    pub fn begin_replace(
        &self,
        prepared: PreparedPngAsset,
    ) -> Result<PngAssetTransaction, PngImportError> {
        let destination = self.path_for(prepared.metric);
        let transaction_directory = self.create_transaction_directory(&destination)?;
        let backup = match move_existing_to_backup(&destination, &transaction_directory) {
            Ok(backup) => backup,
            Err(error) => {
                let _ = fs::remove_dir(&transaction_directory);
                return Err(file_system_error(error));
            }
        };
        if let Err(error) = fs::rename(&prepared.temporary, &destination) {
            if let Some(backup) = &backup {
                let _ = fs::rename(backup, &destination);
            }
            let _ = fs::remove_dir(&transaction_directory);
            return Err(file_system_error(error));
        }
        if let Err(error) = File::open(&self.directory).and_then(|directory| directory.sync_all()) {
            let _ = fs::remove_file(&destination);
            if let Some(backup) = &backup {
                let _ = fs::rename(backup, &destination);
            }
            let _ = fs::remove_dir(&transaction_directory);
            return Err(file_system_error(error));
        }
        Ok(PngAssetTransaction {
            directory: self.directory.clone(),
            destination,
            transaction_directory,
            backup,
            mutation: AssetMutation::Replace,
            active: true,
        })
    }

    pub fn begin_remove(&self, metric: MetricKind) -> Result<PngAssetTransaction, PngImportError> {
        fs::create_dir_all(&self.directory).map_err(file_system_error)?;
        let destination = self.path_for(metric);
        let transaction_directory = self.create_transaction_directory(&destination)?;
        let backup = match move_existing_to_backup(&destination, &transaction_directory) {
            Ok(backup) => backup,
            Err(error) => {
                let _ = fs::remove_dir(&transaction_directory);
                return Err(file_system_error(error));
            }
        };
        if let Err(error) = File::open(&self.directory).and_then(|directory| directory.sync_all()) {
            if let Some(backup) = &backup {
                let _ = fs::rename(backup, &destination);
            }
            let _ = fs::remove_dir(&transaction_directory);
            return Err(file_system_error(error));
        }
        Ok(PngAssetTransaction {
            directory: self.directory.clone(),
            destination,
            transaction_directory,
            backup,
            mutation: AssetMutation::Remove,
            active: true,
        })
    }

    fn write_unique_temporary(
        &self,
        destination: &Path,
        bytes: &[u8],
        suffix: &str,
    ) -> Result<PathBuf, PngImportError> {
        let mut last_collision = None;
        for _ in 0..MAX_TEMPORARY_PATH_ATTEMPTS {
            let temporary = self.temporary_path(
                destination,
                self.temp_sequence.fetch_add(1, Ordering::Relaxed),
                suffix,
            );
            match write_temporary(&temporary, bytes) {
                Ok(()) => return Ok(temporary),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    last_collision = Some(error);
                }
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    return Err(file_system_error(error));
                }
            }
        }
        Err(file_system_error(last_collision.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate an icon temporary file",
            )
        })))
    }

    fn temporary_path(&self, destination: &Path, sequence: u64, suffix: &str) -> PathBuf {
        destination.with_file_name(format!(
            ".{}.{}.{sequence}.{suffix}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("icon.png"),
            std::process::id(),
        ))
    }

    fn create_transaction_directory(&self, destination: &Path) -> Result<PathBuf, PngImportError> {
        let mut last_collision = None;
        for _ in 0..MAX_TEMPORARY_PATH_ATTEMPTS {
            let path = self.temporary_path(
                destination,
                self.temp_sequence.fetch_add(1, Ordering::Relaxed),
                "transaction",
            );
            match fs::create_dir(&path) {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    last_collision = Some(error);
                }
                Err(error) => return Err(file_system_error(error)),
            }
        }
        Err(file_system_error(last_collision.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate an icon transaction directory",
            )
        })))
    }

    pub fn remove(&self, metric: MetricKind) -> Result<(), PngImportError> {
        match fs::remove_file(self.path_for(metric)) {
            Ok(()) => File::open(&self.directory)
                .and_then(|directory| directory.sync_all())
                .map_err(file_system_error),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(file_system_error(error)),
        }
    }

    pub fn read(&self, metric: MetricKind) -> Result<Vec<u8>, PngImportError> {
        let bytes = fs::read(self.path_for(metric)).map_err(file_system_error)?;
        normalize_png("stored.png", &bytes).map(|normalized| normalized.bytes)
    }
}

fn write_temporary(temporary: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn move_existing_to_backup(
    destination: &Path,
    transaction_directory: &Path,
) -> io::Result<Option<PathBuf>> {
    if !destination.exists() {
        return Ok(None);
    }
    let backup = transaction_directory.join("previous.png");
    fs::rename(destination, &backup)?;
    Ok(Some(backup))
}

fn file_system_error(error: io::Error) -> PngImportError {
    PngImportError::new(
        PngImportErrorKind::FileSystem,
        format!("Não foi possível salvar o PNG: {error}"),
    )
}
