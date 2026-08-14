use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader};

use crate::indicator_preferences::{IdentifierPreferences, MetricKind, PngIconMetadata};

pub const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_DIMENSION: u32 = 8_192;
const MAX_SOURCE_PIXELS: u64 = 16_777_216;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TEMPORARY_PATH_ATTEMPTS: usize = 1_024;

trait AssetFileSystem: fmt::Debug + Send + Sync {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn write_temporary(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    fn sync_directory(&self, path: &Path) -> io::Result<()>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

#[derive(Debug)]
struct NativeAssetFileSystem;

impl AssetFileSystem for NativeAssetFileSystem {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn write_temporary(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        write_temporary(path, bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir(path)
    }

    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        File::open(path)?.sync_all()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}

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
    retained_transactions: Vec<PngAssetTransaction>,
}

impl PngImportError {
    fn new(kind: PngImportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retained_transactions: Vec::new(),
        }
    }

    fn with_retained_transaction(mut self, transaction: PngAssetTransaction) -> Self {
        self.retained_transactions.push(transaction);
        self
    }

    fn with_retained_transactions(
        mut self,
        transactions: impl IntoIterator<Item = PngAssetTransaction>,
    ) -> Self {
        self.retained_transactions.extend(transactions);
        self
    }

    pub const fn kind(&self) -> PngImportErrorKind {
        self.kind
    }

    pub fn user_message(&self) -> &str {
        &self.message
    }

    pub fn into_parts(self) -> (String, Vec<PngAssetTransaction>) {
        (self.message, self.retained_transactions)
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
    file_system: Arc<dyn AssetFileSystem>,
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
        if let Err(error) = fs::remove_file(&self.temporary) {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!(
                    "Statlet could not clean a prepared PNG at {}: {error}",
                    self.temporary.display()
                );
            }
        }
    }
}

#[derive(Debug)]
pub struct IndicatorPngSnapshot {
    directory: PathBuf,
    snapshot_directory: Option<PathBuf>,
    retained: [Option<PathBuf>; 2],
    affected_metrics: [bool; 2],
    file_system: Arc<dyn AssetFileSystem>,
    active: bool,
}

#[derive(Debug)]
pub struct IndicatorPngSnapshotCaptureError {
    error: PngImportError,
    retained_snapshot: Option<Box<IndicatorPngSnapshot>>,
}

impl IndicatorPngSnapshotCaptureError {
    fn new(error: PngImportError, retained_snapshot: Option<IndicatorPngSnapshot>) -> Self {
        Self {
            error,
            retained_snapshot: retained_snapshot.map(Box::new),
        }
    }

    pub fn user_message(&self) -> &str {
        self.error.user_message()
    }

    pub const fn has_retained_snapshot(&self) -> bool {
        self.retained_snapshot.is_some()
    }

    pub fn into_parts(self) -> (String, Option<IndicatorPngSnapshot>) {
        (
            self.error.to_string(),
            self.retained_snapshot.map(|snapshot| *snapshot),
        )
    }
}

impl IndicatorPngSnapshot {
    pub fn affected_metrics(&self) -> Vec<MetricKind> {
        [MetricKind::Cpu, MetricKind::Ram]
            .into_iter()
            .filter(|metric| self.affected_metrics[metric_index(*metric)])
            .collect()
    }

    pub fn cleanup(&mut self) -> Result<(), PngImportError> {
        if self.snapshot_directory.is_none()
            && self.retained.iter().all(Option::is_none)
            && !self.directory.exists()
        {
            self.active = false;
            return Ok(());
        }
        let mut failures = Vec::new();
        for retained in &mut self.retained {
            let Some(path) = retained.as_ref() else {
                continue;
            };
            match self.file_system.remove_file(path) {
                Ok(()) => *retained = None,
                Err(error) if error.kind() == io::ErrorKind::NotFound => *retained = None,
                Err(error) => failures.push(format!(
                    "remover o asset retido em {}: {error}",
                    path.display()
                )),
            }
        }
        if self.retained.iter().all(Option::is_none) {
            if let Some(snapshot_directory) = self.snapshot_directory.as_ref() {
                match self.file_system.remove_dir(snapshot_directory) {
                    Ok(()) => self.snapshot_directory = None,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        self.snapshot_directory = None
                    }
                    Err(error) => failures.push(format!(
                        "limpar o snapshot em {}: {error}",
                        snapshot_directory.display()
                    )),
                }
            }
        }
        record_operation_failure(
            &mut failures,
            "sincronizar o diretório",
            &self.directory,
            self.file_system.sync_directory(&self.directory),
            false,
        );
        let result = transaction_result(
            "Não foi possível limpar o snapshot de Undo dos PNGs",
            failures,
        );
        if result.is_ok() {
            self.active = false;
        }
        result
    }

    fn retained_bytes(
        &self,
        metric: MetricKind,
        metadata: &PngIconMetadata,
    ) -> Result<Vec<u8>, PngImportError> {
        let path = self.retained[metric_index(metric)]
            .as_ref()
            .ok_or_else(|| {
                PngImportError::new(
                    PngImportErrorKind::FileSystem,
                    format!("O snapshot de Undo não contém o PNG de {metric:?}."),
                )
            })?;
        let bytes = self.file_system.read(path).map_err(file_system_error)?;
        validate_snapshot_bytes(metadata, &bytes)?;
        Ok(bytes)
    }
}

impl Drop for IndicatorPngSnapshot {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.cleanup() {
                eprintln!("Statlet could not clean an abandoned PNG undo snapshot: {error}");
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetMutation {
    Replace,
    Remove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetTransactionResolution {
    Rollback,
    CommitCleanup,
    Complete,
}

#[derive(Debug)]
pub struct PngAssetTransaction {
    directory: PathBuf,
    destination: PathBuf,
    transaction_directory: PathBuf,
    backup: Option<PathBuf>,
    mutation: AssetMutation,
    replacement_removed: bool,
    resolution: AssetTransactionResolution,
    file_system: Arc<dyn AssetFileSystem>,
}

impl PngAssetTransaction {
    pub fn commit(mut self) -> Result<(), PngImportError> {
        self.resolution = AssetTransactionResolution::CommitCleanup;
        match self.commit_inner() {
            Ok(()) => {
                self.resolution = AssetTransactionResolution::Complete;
                Ok(())
            }
            Err(error) => Err(error.with_retained_transaction(self)),
        }
    }

    fn commit_inner(&mut self) -> Result<(), PngImportError> {
        let mut failures = Vec::new();
        if let Some(backup) = &self.backup {
            record_operation_failure(
                &mut failures,
                "remover o backup confirmado",
                backup,
                self.file_system.remove_file(backup),
                true,
            );
        }
        record_operation_failure(
            &mut failures,
            "limpar a transação",
            &self.transaction_directory,
            self.file_system.remove_dir(&self.transaction_directory),
            true,
        );
        record_operation_failure(
            &mut failures,
            "sincronizar o diretório",
            &self.directory,
            self.file_system.sync_directory(&self.directory),
            false,
        );
        transaction_result("Não foi possível concluir a transação do PNG", failures)
    }

    pub fn rollback(mut self) -> Result<(), PngImportError> {
        self.resolution = AssetTransactionResolution::Rollback;
        match self.rollback_inner() {
            Ok(()) => {
                self.resolution = AssetTransactionResolution::Complete;
                Ok(())
            }
            Err(error) => Err(error.with_retained_transaction(self)),
        }
    }

    fn rollback_inner(&mut self) -> Result<(), PngImportError> {
        let mut failures = Vec::new();
        if self.mutation == AssetMutation::Replace && !self.replacement_removed {
            match self.file_system.remove_file(&self.destination) {
                Ok(()) => self.replacement_removed = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    self.replacement_removed = true
                }
                Err(error) => failures.push(format!(
                    "remover o PNG novo em {}: {error}",
                    self.destination.display()
                )),
            }
        }
        if let Some(backup) = self.backup.clone() {
            match self.file_system.rename(&backup, &self.destination) {
                Ok(()) => self.backup = None,
                Err(error) => failures.push(format!(
                    "restaurar o PNG anterior em {}: {error}",
                    backup.display()
                )),
            }
        }
        record_operation_failure(
            &mut failures,
            "limpar a transação",
            &self.transaction_directory,
            self.file_system.remove_dir(&self.transaction_directory),
            true,
        );
        record_operation_failure(
            &mut failures,
            "sincronizar o diretório",
            &self.directory,
            self.file_system.sync_directory(&self.directory),
            false,
        );
        transaction_result("Não foi possível restaurar a transação do PNG", failures)
    }
}

impl Drop for PngAssetTransaction {
    fn drop(&mut self) {
        let result = match self.resolution {
            AssetTransactionResolution::Rollback => self.rollback_inner(),
            AssetTransactionResolution::CommitCleanup => self.commit_inner(),
            AssetTransactionResolution::Complete => Ok(()),
        };
        if let Err(error) = result {
            eprintln!("Statlet could not finish an abandoned PNG transaction: {error}");
        }
    }
}

impl IconAssetStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            temp_sequence: Arc::new(AtomicU64::new(0)),
            file_system: Arc::new(NativeAssetFileSystem),
        }
    }

    #[cfg(test)]
    fn with_file_system(directory: PathBuf, file_system: Arc<dyn AssetFileSystem>) -> Self {
        Self {
            directory,
            temp_sequence: Arc::new(AtomicU64::new(0)),
            file_system,
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
        self.begin_replace(prepared)?.commit()?;
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

    pub fn capture_indicator_snapshot(
        &self,
        identifiers: &IdentifierPreferences,
    ) -> Result<IndicatorPngSnapshot, IndicatorPngSnapshotCaptureError> {
        let metadata = [
            (MetricKind::Cpu, identifiers.cpu.png.as_ref()),
            (MetricKind::Ram, identifiers.ram.png.as_ref()),
        ];
        let mut snapshot = IndicatorPngSnapshot {
            directory: self.directory.clone(),
            snapshot_directory: None,
            retained: [None, None],
            affected_metrics: [metadata[0].1.is_some(), metadata[1].1.is_some()],
            file_system: self.file_system.clone(),
            active: true,
        };
        if metadata.iter().all(|(_, metadata)| metadata.is_none()) {
            return Ok(snapshot);
        }
        fs::create_dir_all(&self.directory)
            .map_err(file_system_error)
            .map_err(|error| IndicatorPngSnapshotCaptureError::new(error, None))?;
        let snapshot_directory = self
            .create_transaction_directory(&self.path_for(MetricKind::Cpu))
            .map_err(|error| IndicatorPngSnapshotCaptureError::new(error, None))?;
        snapshot.snapshot_directory = Some(snapshot_directory.clone());
        for (metric, metadata) in metadata {
            let Some(metadata) = metadata else {
                continue;
            };
            let result = (|| {
                let bytes = self
                    .file_system
                    .read(&self.path_for(metric))
                    .map_err(file_system_error)?;
                validate_snapshot_bytes(metadata, &bytes)?;
                let retained = snapshot_directory.join(match metric {
                    MetricKind::Cpu => "cpu.png",
                    MetricKind::Ram => "ram.png",
                });
                snapshot.retained[metric_index(metric)] = Some(retained.clone());
                self.file_system
                    .write_temporary(&retained, &bytes)
                    .map_err(file_system_error)?;
                Ok::<(), PngImportError>(())
            })();
            if let Err(error) = result {
                let cleanup = snapshot.cleanup().err();
                let retained_snapshot = cleanup.as_ref().map(|_| snapshot);
                return Err(IndicatorPngSnapshotCaptureError::new(
                    PngImportError::new(
                        PngImportErrorKind::FileSystem,
                        cleanup.map_or_else(
                            || error.to_string(),
                            |cleanup| format!("{error}; a compensação também falhou: {cleanup}"),
                        ),
                    ),
                    retained_snapshot,
                ));
            }
        }
        if let Err(error) = self.file_system.sync_directory(&self.directory) {
            let error = file_system_error(error);
            let cleanup = snapshot.cleanup().err();
            let retained_snapshot = cleanup.as_ref().map(|_| snapshot);
            return Err(IndicatorPngSnapshotCaptureError::new(
                PngImportError::new(
                    PngImportErrorKind::FileSystem,
                    cleanup.map_or_else(
                        || error.to_string(),
                        |cleanup| format!("{error}; a compensação também falhou: {cleanup}"),
                    ),
                ),
                retained_snapshot,
            ));
        }
        Ok(snapshot)
    }

    pub fn begin_restore_indicator_snapshot(
        &self,
        snapshot: &IndicatorPngSnapshot,
        identifiers: &IdentifierPreferences,
    ) -> Result<Vec<PngAssetTransaction>, PngImportError> {
        let targets = [
            (MetricKind::Cpu, identifiers.cpu.png.as_ref()),
            (MetricKind::Ram, identifiers.ram.png.as_ref()),
        ];
        let mut transactions = Vec::with_capacity(2);
        for (metric, metadata) in targets {
            let transaction = (|| match metadata {
                Some(metadata) => {
                    let bytes = snapshot.retained_bytes(metric, metadata)?;
                    let destination = self.path_for(metric);
                    let temporary = self.write_unique_temporary(&destination, &bytes, "undo")?;
                    self.begin_replace(PreparedPngAsset {
                        metric,
                        temporary,
                        metadata: metadata.clone(),
                    })
                }
                None => self.begin_remove(metric),
            })();
            match transaction {
                Ok(transaction) => transactions.push(transaction),
                Err(error) => {
                    let (primary, mut retained_transactions) = error.into_parts();
                    let mut rollback_failures = Vec::new();
                    for transaction in transactions.into_iter().rev() {
                        if let Err(error) = transaction.rollback() {
                            let (message, retained) = error.into_parts();
                            rollback_failures.push(message);
                            retained_transactions.extend(retained);
                        }
                    }
                    return Err(PngImportError::new(
                        PngImportErrorKind::FileSystem,
                        if rollback_failures.is_empty() {
                            primary
                        } else {
                            format!(
                                "{primary}; a compensação também falhou: {}",
                                rollback_failures.join("; ")
                            )
                        },
                    )
                    .with_retained_transactions(retained_transactions));
                }
            }
        }
        Ok(transactions)
    }

    pub fn begin_replace(
        &self,
        prepared: PreparedPngAsset,
    ) -> Result<PngAssetTransaction, PngImportError> {
        let destination = self.path_for(prepared.metric);
        let transaction_directory = self.create_transaction_directory(&destination)?;
        let backup = match move_existing_to_backup(
            self.file_system.as_ref(),
            &destination,
            &transaction_directory,
        ) {
            Ok(backup) => backup,
            Err(error) => {
                let cleanup = self
                    .cleanup_empty_transaction(&transaction_directory)
                    .map_err(|cleanup| {
                        cleanup.with_retained_transaction(self.cleanup_transaction(
                            destination.clone(),
                            transaction_directory.clone(),
                        ))
                    });
                return Err(compensated_operation_error(
                    "Não foi possível preparar o backup do PNG",
                    error,
                    cleanup,
                ));
            }
        };
        if let Err(error) = self.file_system.rename(&prepared.temporary, &destination) {
            let transaction = self.transaction(
                destination,
                transaction_directory,
                backup,
                AssetMutation::Replace,
            );
            let rollback = transaction.rollback();
            return Err(compensated_operation_error(
                "Não foi possível instalar o PNG",
                error,
                rollback,
            ));
        }
        if let Err(error) = self.file_system.sync_directory(&self.directory) {
            let transaction = self.transaction(
                destination,
                transaction_directory,
                backup,
                AssetMutation::Replace,
            );
            let rollback = transaction.rollback();
            return Err(compensated_operation_error(
                "Não foi possível instalar o PNG",
                error,
                rollback,
            ));
        }
        Ok(self.transaction(
            destination,
            transaction_directory,
            backup,
            AssetMutation::Replace,
        ))
    }

    pub fn begin_remove(&self, metric: MetricKind) -> Result<PngAssetTransaction, PngImportError> {
        fs::create_dir_all(&self.directory).map_err(file_system_error)?;
        let destination = self.path_for(metric);
        let transaction_directory = self.create_transaction_directory(&destination)?;
        let backup = match move_existing_to_backup(
            self.file_system.as_ref(),
            &destination,
            &transaction_directory,
        ) {
            Ok(backup) => backup,
            Err(error) => {
                let cleanup = self
                    .cleanup_empty_transaction(&transaction_directory)
                    .map_err(|cleanup| {
                        cleanup.with_retained_transaction(self.cleanup_transaction(
                            destination.clone(),
                            transaction_directory.clone(),
                        ))
                    });
                return Err(compensated_operation_error(
                    "Não foi possível preparar a remoção do PNG",
                    error,
                    cleanup,
                ));
            }
        };
        if let Err(error) = self.file_system.sync_directory(&self.directory) {
            let transaction = self.transaction(
                destination,
                transaction_directory,
                backup,
                AssetMutation::Remove,
            );
            let rollback = transaction.rollback();
            return Err(compensated_operation_error(
                "Não foi possível remover o PNG",
                error,
                rollback,
            ));
        }
        Ok(self.transaction(
            destination,
            transaction_directory,
            backup,
            AssetMutation::Remove,
        ))
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
            match self.file_system.write_temporary(&temporary, bytes) {
                Ok(()) => return Ok(temporary),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    last_collision = Some(error);
                }
                Err(error) => {
                    let mut failures = Vec::new();
                    record_operation_failure(
                        &mut failures,
                        "limpar o PNG temporário incompleto",
                        &temporary,
                        self.file_system.remove_file(&temporary),
                        true,
                    );
                    let cleanup = transaction_result(
                        "Não foi possível limpar o PNG temporário incompleto",
                        failures,
                    );
                    return Err(compensated_operation_error(
                        "Não foi possível gravar o PNG temporário",
                        error,
                        cleanup,
                    ));
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
            match self.file_system.create_dir(&path) {
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

    fn transaction(
        &self,
        destination: PathBuf,
        transaction_directory: PathBuf,
        backup: Option<PathBuf>,
        mutation: AssetMutation,
    ) -> PngAssetTransaction {
        PngAssetTransaction {
            directory: self.directory.clone(),
            destination,
            transaction_directory,
            backup,
            mutation,
            replacement_removed: false,
            resolution: AssetTransactionResolution::Rollback,
            file_system: self.file_system.clone(),
        }
    }

    fn cleanup_transaction(
        &self,
        destination: PathBuf,
        transaction_directory: PathBuf,
    ) -> PngAssetTransaction {
        PngAssetTransaction {
            directory: self.directory.clone(),
            destination,
            transaction_directory,
            backup: None,
            mutation: AssetMutation::Remove,
            replacement_removed: true,
            resolution: AssetTransactionResolution::CommitCleanup,
            file_system: self.file_system.clone(),
        }
    }

    fn cleanup_empty_transaction(
        &self,
        transaction_directory: &Path,
    ) -> Result<(), PngImportError> {
        let mut failures = Vec::new();
        record_operation_failure(
            &mut failures,
            "limpar a transação",
            transaction_directory,
            self.file_system.remove_dir(transaction_directory),
            true,
        );
        record_operation_failure(
            &mut failures,
            "sincronizar o diretório",
            &self.directory,
            self.file_system.sync_directory(&self.directory),
            false,
        );
        transaction_result("Não foi possível limpar a transação do PNG", failures)
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
    file_system: &dyn AssetFileSystem,
    destination: &Path,
    transaction_directory: &Path,
) -> io::Result<Option<PathBuf>> {
    if !destination.exists() {
        return Ok(None);
    }
    let backup = transaction_directory.join("previous.png");
    file_system.rename(destination, &backup)?;
    Ok(Some(backup))
}

fn record_operation_failure(
    failures: &mut Vec<String>,
    operation: &str,
    path: &Path,
    result: io::Result<()>,
    ignore_not_found: bool,
) {
    if let Err(error) = result {
        if ignore_not_found && error.kind() == io::ErrorKind::NotFound {
            return;
        }
        failures.push(format!("{operation} em {}: {error}", path.display()));
    }
}

fn transaction_result(summary: &str, failures: Vec<String>) -> Result<(), PngImportError> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(PngImportError::new(
            PngImportErrorKind::FileSystem,
            format!("{summary}: {}", failures.join("; ")),
        ))
    }
}

fn compensated_operation_error(
    operation: &str,
    primary: io::Error,
    compensation: Result<(), PngImportError>,
) -> PngImportError {
    let (compensation, retained_transactions) = compensation.err().map_or_else(
        || (String::new(), Vec::new()),
        |error| {
            let (message, retained_transactions) = error.into_parts();
            (
                format!("; a compensação também falhou: {message}"),
                retained_transactions,
            )
        },
    );
    PngImportError::new(
        PngImportErrorKind::FileSystem,
        format!("{operation}: {primary}{compensation}"),
    )
    .with_retained_transactions(retained_transactions)
}

fn file_system_error(error: io::Error) -> PngImportError {
    PngImportError::new(
        PngImportErrorKind::FileSystem,
        format!("Não foi possível salvar o PNG: {error}"),
    )
}

fn metric_index(metric: MetricKind) -> usize {
    match metric {
        MetricKind::Cpu => 0,
        MetricKind::Ram => 1,
    }
}

fn validate_snapshot_bytes(metadata: &PngIconMetadata, bytes: &[u8]) -> Result<(), PngImportError> {
    let length_matches = metadata.byte_length() == bytes.len() as u64;
    let fingerprint_matches =
        metadata.content_fingerprint() == 0 || metadata.content_fingerprint() == fnv1a(bytes);
    if length_matches && fingerprint_matches {
        Ok(())
    } else {
        Err(PngImportError::new(
            PngImportErrorKind::FileSystem,
            "O PNG ativo não corresponde à metadata salva; o reset foi cancelado.",
        ))
    }
}

#[cfg(test)]
mod fault_injection_tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use tempfile::tempdir;

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FaultPoint {
        CreateDirectory,
        WriteTemporary,
        Rename,
        RemoveFile,
        RemoveDirectory,
        SyncDirectory,
    }

    #[derive(Debug, Default)]
    struct FaultInjectingFileSystem {
        failures: Mutex<VecDeque<FaultPoint>>,
        calls: Mutex<Vec<FaultPoint>>,
    }

    impl FaultInjectingFileSystem {
        fn arm(&self, failures: impl IntoIterator<Item = FaultPoint>) {
            self.failures.lock().unwrap().extend(failures);
        }

        fn record(&self, point: FaultPoint) -> io::Result<()> {
            self.calls.lock().unwrap().push(point);
            let mut failures = self.failures.lock().unwrap();
            if failures.front() == Some(&point) {
                failures.pop_front();
                Err(io::Error::other(format!("fault injected at {point:?}")))
            } else {
                Ok(())
            }
        }
    }

    impl AssetFileSystem for FaultInjectingFileSystem {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.record(FaultPoint::CreateDirectory)?;
            fs::create_dir(path)
        }

        fn write_temporary(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            if let Err(error) = self.record(FaultPoint::WriteTemporary) {
                fs::write(path, b"partial PNG from injected write failure")?;
                return Err(error);
            }
            super::write_temporary(path, bytes)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.record(FaultPoint::Rename)?;
            fs::rename(from, to)
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.record(FaultPoint::RemoveFile)?;
            fs::remove_file(path)
        }

        fn remove_dir(&self, path: &Path) -> io::Result<()> {
            self.record(FaultPoint::RemoveDirectory)?;
            fs::remove_dir(path)
        }

        fn sync_directory(&self, path: &Path) -> io::Result<()> {
            self.record(FaultPoint::SyncDirectory)?;
            File::open(path)?.sync_all()
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            fs::read(path)
        }
    }

    #[test]
    fn rollback_attempts_every_compensation_step_and_reports_all_failures() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        native_store
            .import_bytes(MetricKind::Cpu, "old.png", &png([0x11, 0x22, 0x33, 0xFF]))
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let prepared = store
            .prepare_bytes(MetricKind::Cpu, "new.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        let transaction = store.begin_replace(prepared).unwrap();
        file_system.arm([
            FaultPoint::RemoveFile,
            FaultPoint::Rename,
            FaultPoint::RemoveDirectory,
            FaultPoint::SyncDirectory,
        ]);

        let error = transaction.rollback().unwrap_err();

        assert!(error.user_message().contains("remover o PNG novo"));
        assert!(error.user_message().contains("restaurar o PNG anterior"));
        assert!(error.user_message().contains("limpar a transação"));
        assert!(error.user_message().contains("sincronizar o diretório"));
        assert!(file_system.failures.lock().unwrap().is_empty());
        let (_, mut retained) = error.into_parts();
        assert_eq!(retained.len(), 1);
        retained.pop().unwrap().rollback().unwrap();
        assert_eq!(
            fs::read(store.path_for(MetricKind::Cpu)).unwrap(),
            png([0x11, 0x22, 0x33, 0xFF])
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn rollback_retry_does_not_repeat_an_already_restored_backup() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        let original = png([0x11, 0x22, 0x33, 0xFF]);
        native_store
            .import_bytes(MetricKind::Cpu, "old.png", &original)
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let prepared = store
            .prepare_bytes(MetricKind::Cpu, "new.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        let transaction = store.begin_replace(prepared).unwrap();
        file_system.arm([FaultPoint::SyncDirectory]);

        let error = transaction.rollback().unwrap_err();
        let (_, mut retained) = error.into_parts();
        assert_eq!(retained.len(), 1);
        assert!(retained[0].backup.is_none());

        retained.pop().unwrap().rollback().unwrap();

        assert_eq!(fs::read(store.path_for(MetricKind::Cpu)).unwrap(), original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn begin_replace_propagates_failures_from_its_disk_compensation() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        native_store
            .import_bytes(MetricKind::Cpu, "old.png", &png([0x11, 0x22, 0x33, 0xFF]))
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let prepared = store
            .prepare_bytes(MetricKind::Cpu, "new.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        file_system.arm([
            FaultPoint::SyncDirectory,
            FaultPoint::Rename,
            FaultPoint::RemoveDirectory,
        ]);

        let error = store.begin_replace(prepared).unwrap_err();

        assert!(error.user_message().contains("instalar o PNG"));
        assert!(error.user_message().contains("restaurar o PNG anterior"));
        assert!(error.user_message().contains("limpar a transação"));
        assert!(file_system.failures.lock().unwrap().is_empty());
    }

    #[test]
    fn backup_prepare_failure_retains_empty_transaction_cleanup_owner() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        let original = png([0x11, 0x22, 0x33, 0xFF]);
        native_store
            .import_bytes(MetricKind::Cpu, "old.png", &original)
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let prepared = store
            .prepare_bytes(MetricKind::Cpu, "new.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        file_system.arm([FaultPoint::Rename, FaultPoint::RemoveDirectory]);

        let error = store.begin_replace(prepared).unwrap_err();
        let (_, mut retained) = error.into_parts();

        assert_eq!(retained.len(), 1);
        retained.pop().unwrap().commit().unwrap();
        assert_eq!(fs::read(store.path_for(MetricKind::Cpu)).unwrap(), original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn commit_propagates_backup_cleanup_and_directory_sync_failures() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        native_store
            .import_bytes(MetricKind::Cpu, "old.png", &png([0x11, 0x22, 0x33, 0xFF]))
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let prepared = store
            .prepare_bytes(MetricKind::Cpu, "new.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap();
        let transaction = store.begin_replace(prepared).unwrap();
        file_system.arm([
            FaultPoint::RemoveFile,
            FaultPoint::RemoveDirectory,
            FaultPoint::SyncDirectory,
        ]);

        let error = transaction.commit().unwrap_err();

        assert!(error.user_message().contains("remover o backup confirmado"));
        assert!(error.user_message().contains("limpar a transação"));
        assert!(error.user_message().contains("sincronizar o diretório"));
        assert!(file_system.failures.lock().unwrap().is_empty());
        let (_, mut retained) = error.into_parts();
        assert_eq!(retained.len(), 1);
        retained.pop().unwrap().commit().unwrap();
        assert_eq!(
            fs::read(store.path_for(MetricKind::Cpu)).unwrap(),
            png([0xAA, 0xBB, 0xCC, 0xFF])
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn retained_snapshot_restores_original_hash_after_replacement_and_removal() {
        let directory = tempdir().unwrap();
        let store = IconAssetStore::new(directory.path().to_path_buf());
        let original_cpu = png([0x11, 0x22, 0x33, 0xFF]);
        let original_ram = png([0x44, 0x55, 0x66, 0xFF]);
        let replacement = png([0xAA, 0xBB, 0xCC, 0xFF]);
        let cpu_metadata = store
            .import_bytes(MetricKind::Cpu, "original-cpu.png", &original_cpu)
            .unwrap();
        let ram_metadata = store
            .import_bytes(MetricKind::Ram, "original-ram.png", &original_ram)
            .unwrap();
        let original_cpu_bytes = fs::read(store.path_for(MetricKind::Cpu)).unwrap();
        let original_ram_bytes = fs::read(store.path_for(MetricKind::Ram)).unwrap();
        let mut identifiers = IdentifierPreferences::default();
        identifiers.cpu.png = Some(cpu_metadata);
        identifiers.ram.png = Some(ram_metadata);
        let mut snapshot = store.capture_indicator_snapshot(&identifiers).unwrap();
        let prepared = store
            .prepare_bytes(MetricKind::Cpu, "replacement.png", &replacement)
            .unwrap();
        store.begin_replace(prepared).unwrap().commit().unwrap();
        store
            .begin_remove(MetricKind::Cpu)
            .unwrap()
            .commit()
            .unwrap();
        store
            .begin_remove(MetricKind::Ram)
            .unwrap()
            .commit()
            .unwrap();

        let transactions = store
            .begin_restore_indicator_snapshot(&snapshot, &identifiers)
            .unwrap();
        for transaction in transactions {
            transaction.commit().unwrap();
        }
        snapshot.cleanup().unwrap();

        let restored_cpu = fs::read(store.path_for(MetricKind::Cpu)).unwrap();
        let restored_ram = fs::read(store.path_for(MetricKind::Ram)).unwrap();
        assert_eq!(restored_cpu, original_cpu_bytes);
        assert_eq!(restored_ram, original_ram_bytes);
        assert_eq!(
            fnv1a(&restored_cpu),
            identifiers.cpu.png.unwrap().content_fingerprint()
        );
        assert_eq!(
            fnv1a(&restored_ram),
            identifiers.ram.png.unwrap().content_fingerprint()
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn partial_multi_metric_snapshot_restore_returns_every_failed_compensation_owner() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        let ram_metadata = native_store
            .import_bytes(
                MetricKind::Ram,
                "original-ram.png",
                &png([0x44, 0x55, 0x66, 0xFF]),
            )
            .unwrap();
        let mut identifiers = IdentifierPreferences::default();
        identifiers.ram.png = Some(ram_metadata);
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let mut snapshot = store.capture_indicator_snapshot(&identifiers).unwrap();
        store
            .begin_remove(MetricKind::Ram)
            .unwrap()
            .commit()
            .unwrap();
        file_system.arm([FaultPoint::Rename, FaultPoint::RemoveDirectory]);

        let error = store
            .begin_restore_indicator_snapshot(&snapshot, &identifiers)
            .unwrap_err();
        let (message, mut retained) = error.into_parts();

        assert!(message.contains("instalar o PNG"));
        assert!(message.contains("compensação"));
        assert_eq!(retained.len(), 1);
        retained.pop().unwrap().rollback().unwrap();
        assert!(!store.path_for(MetricKind::Cpu).exists());
        assert!(!store.path_for(MetricKind::Ram).exists());
        snapshot.cleanup().unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn failed_snapshot_cleanup_retains_ownership_and_succeeds_on_retry() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        let metadata = native_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let mut identifiers = IdentifierPreferences::default();
        identifiers.cpu.png = Some(metadata);
        let mut snapshot = store.capture_indicator_snapshot(&identifiers).unwrap();
        file_system.arm([FaultPoint::RemoveFile]);

        let error = snapshot.cleanup().unwrap_err();

        assert!(error.user_message().contains("asset retido"));
        assert!(snapshot.active);
        assert!(snapshot.retained[metric_index(MetricKind::Cpu)].is_some());
        snapshot.cleanup().unwrap();
        assert!(!snapshot.active);
        assert!(snapshot.retained.iter().all(Option::is_none));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn capture_failure_returns_the_partial_snapshot_when_compensation_fails() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        let metadata = native_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let mut identifiers = IdentifierPreferences::default();
        identifiers.cpu.png = Some(metadata);
        file_system.arm([FaultPoint::SyncDirectory, FaultPoint::RemoveFile]);

        let failure = store.capture_indicator_snapshot(&identifiers).unwrap_err();

        assert!(failure.user_message().contains("compensação"));
        assert!(failure.has_retained_snapshot());
    }

    #[test]
    fn partial_snapshot_write_retains_path_ownership_until_cleanup_retry_succeeds() {
        let directory = tempdir().unwrap();
        let native_store = IconAssetStore::new(directory.path().to_path_buf());
        let metadata = native_store
            .import_bytes(
                MetricKind::Cpu,
                "original.png",
                &png([0x11, 0x22, 0x33, 0xFF]),
            )
            .unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        let mut identifiers = IdentifierPreferences::default();
        identifiers.cpu.png = Some(metadata);
        file_system.arm([FaultPoint::WriteTemporary, FaultPoint::RemoveFile]);

        let failure = store.capture_indicator_snapshot(&identifiers).unwrap_err();
        let (_, retained_snapshot) = failure.into_parts();
        let mut retained_snapshot = retained_snapshot.expect("partial file keeps an owner");

        assert!(retained_snapshot.retained[metric_index(MetricKind::Cpu)].is_some());
        assert!(retained_snapshot.active);
        retained_snapshot.cleanup().unwrap();
        assert!(!retained_snapshot.active);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn empty_snapshot_needs_no_directory_and_cleans_without_filesystem_work() {
        let directory = tempdir().unwrap();
        let icons = directory.path().join("icons");
        let store = IconAssetStore::new(icons.clone());
        let mut snapshot = store
            .capture_indicator_snapshot(&IdentifierPreferences::default())
            .unwrap();

        snapshot.cleanup().unwrap();

        assert!(!icons.exists());
        assert!(!snapshot.active);
    }

    #[test]
    fn failed_temporary_write_propagates_its_cleanup_failure() {
        let directory = tempdir().unwrap();
        let file_system = Arc::new(FaultInjectingFileSystem::default());
        let store =
            IconAssetStore::with_file_system(directory.path().to_path_buf(), file_system.clone());
        file_system.arm([FaultPoint::WriteTemporary, FaultPoint::RemoveFile]);

        let error = store
            .prepare_bytes(MetricKind::Cpu, "new.png", &png([0xAA, 0xBB, 0xCC, 0xFF]))
            .unwrap_err();

        assert!(error.user_message().contains("gravar o PNG temporário"));
        assert!(error
            .user_message()
            .contains("limpar o PNG temporário incompleto"));
        assert!(file_system.failures.lock().unwrap().is_empty());
    }

    fn png(color: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(12, 12, Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
}
