use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use statlet::icon_assets::{normalize_png, IconAssetStore, PngImportErrorKind};
use statlet::indicator_preferences::{MetricKind, PngIconMetadata};
use tempfile::tempdir;

#[test]
fn valid_png_is_downscaled_for_retina_without_losing_aspect_ratio_or_alpha() {
    let source = png(48, 24, [0x22, 0x88, 0xCC, 0x77]);

    let normalized = normalize_png("cpu-wide.png", &source).unwrap();
    let decoded =
        image::load_from_memory_with_format(normalized.bytes(), ImageFormat::Png).unwrap();

    assert_eq!(decoded.dimensions(), (24, 12));
    assert_eq!(decoded.to_rgba8().get_pixel(0, 0).0[3], 0x77);
    assert_eq!(normalized.metadata().source_name(), "cpu-wide.png");
    assert_eq!(normalized.metadata().width(), 24);
    assert_eq!(normalized.metadata().height(), 12);
    assert_eq!(
        normalized.metadata().byte_length(),
        normalized.bytes().len() as u64
    );
}

#[test]
fn small_png_is_reencoded_but_never_upscaled() {
    let source = png(8, 4, [0xFF, 0x80, 0x20, 0xFF]);

    let normalized = normalize_png("tiny.PNG", &source).unwrap();
    let decoded =
        image::load_from_memory_with_format(normalized.bytes(), ImageFormat::Png).unwrap();

    assert_eq!(decoded.dimensions(), (8, 4));
}

#[test]
fn invalid_or_defensively_oversized_png_is_rejected_before_replacement() {
    let directory = tempdir().unwrap();
    let store = IconAssetStore::new(directory.path().to_path_buf());
    let original = png(12, 12, [0x10, 0x20, 0x30, 0xFF]);
    store
        .import_bytes(MetricKind::Cpu, "original.png", &original)
        .unwrap();
    let saved_before = std::fs::read(store.path_for(MetricKind::Cpu)).unwrap();

    let invalid = store
        .import_bytes(MetricKind::Cpu, "broken.png", b"not a png")
        .unwrap_err();
    assert_eq!(invalid.kind(), PngImportErrorKind::InvalidPng);
    assert_eq!(
        std::fs::read(store.path_for(MetricKind::Cpu)).unwrap(),
        saved_before
    );

    let oversized = vec![0; statlet::icon_assets::MAX_SOURCE_BYTES + 1];
    let error = normalize_png("huge.png", &oversized).unwrap_err();
    assert_eq!(error.kind(), PngImportErrorKind::SourceTooLarge);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn suspicious_pixel_dimensions_are_rejected_without_decoding_the_payload() {
    let mut source = png(1, 1, [0, 0, 0, 0]);
    source[16..20].copy_from_slice(&100_000_u32.to_be_bytes());
    source[20..24].copy_from_slice(&100_000_u32.to_be_bytes());

    let error = normalize_png("bomb.png", &source).unwrap_err();

    assert_eq!(error.kind(), PngImportErrorKind::PixelLimitExceeded);
}

#[test]
fn metric_assets_use_stable_names_and_remove_only_the_requested_metric() {
    let directory = tempdir().unwrap();
    let store = IconAssetStore::new(directory.path().to_path_buf());
    let cpu = png(12, 12, [0x11, 0x22, 0x33, 0xFF]);
    let ram = png(12, 12, [0x44, 0x55, 0x66, 0xFF]);

    store
        .import_bytes(MetricKind::Cpu, "first.png", &cpu)
        .unwrap();
    store
        .import_bytes(MetricKind::Ram, "second.png", &ram)
        .unwrap();

    assert_eq!(
        store.path_for(MetricKind::Cpu).file_name().unwrap(),
        "cpu.png"
    );
    assert_eq!(
        store.path_for(MetricKind::Ram).file_name().unwrap(),
        "ram.png"
    );
    store.remove(MetricKind::Cpu).unwrap();
    assert!(!store.path_for(MetricKind::Cpu).exists());
    assert!(store.path_for(MetricKind::Ram).exists());
}

#[test]
fn metadata_rejects_control_characters_in_the_display_name() {
    assert!(PngIconMetadata::new("icone\nenganoso.png", 12, 12, 100).is_err());
}

#[test]
fn png_metadata_fingerprint_changes_when_equal_sized_visual_content_changes() {
    let first = normalize_png("same.png", &png(12, 12, [0x11, 0x22, 0x33, 0xFF])).unwrap();
    let second = normalize_png("same.png", &png(12, 12, [0x44, 0x55, 0x66, 0xFF])).unwrap();

    assert_eq!(first.metadata().width(), second.metadata().width());
    assert_eq!(first.metadata().height(), second.metadata().height());
    assert_ne!(
        first.metadata().content_fingerprint(),
        second.metadata().content_fingerprint()
    );
}

#[test]
fn stale_temporary_file_from_an_interrupted_process_does_not_block_import() {
    let directory = tempdir().unwrap();
    let store = IconAssetStore::new(directory.path().to_path_buf());
    let stale = directory
        .path()
        .join(format!(".cpu.png.{}.tmp", std::process::id()));
    std::fs::write(&stale, b"stale").unwrap();

    let result = store.import_bytes(
        MetricKind::Cpu,
        "replacement.png",
        &png(12, 12, [0xAA, 0xBB, 0xCC, 0xFF]),
    );

    assert!(result.is_ok());
    assert!(store.path_for(MetricKind::Cpu).exists());
}

fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba(color));
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
