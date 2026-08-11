use statlet::disk::format_decimal_gigabytes;

#[test]
fn capacity_uses_decimal_gigabytes_like_macos_storage_ui() {
    assert_eq!(format_decimal_gigabytes(34_800_000_000), "34.8 GB");
}
