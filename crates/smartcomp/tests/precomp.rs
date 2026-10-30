use std::io::Write;

use flate2::{write::GzEncoder, Compression};
use smartcomp::codecs::{by_id, pack, unpack, Level};

/// Precomp runs in a child process of the smartcomp executable; point the codec at it.
fn use_built_binary() {
    std::env::set_var("SMARTCOMP_EXE", env!("CARGO_BIN_EXE_smartcomp"));
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[test]
fn gzip_file_shrinks_and_restores_exactly() {
    use_built_binary();
    let text: Vec<u8> = (0..4000).flat_map(|i| format!("line {i}: the quick brown fox {}\n", i * 7 % 13).into_bytes()).collect();
    let gz = gzip(&text);
    let codec = by_id("precomp").unwrap();
    for level in [Level::Medium, Level::Ultra, Level::HighUltra] {
        let packed = pack(codec, &gz, level).unwrap();
        assert!(packed.len() < gz.len(), "{level:?}: {} -> {}", gz.len(), packed.len());
        assert_eq!(unpack(codec, &packed, gz.len()).unwrap(), gz, "{level:?}");
    }
}

#[test]
fn data_without_streams_is_still_lossless() {
    use_built_binary();
    let plain = b"no deflate streams in here, just text ".repeat(50);
    let codec = by_id("precomp").unwrap();
    let packed = pack(codec, &plain, Level::Medium).unwrap();
    assert_eq!(unpack(codec, &packed, plain.len()).unwrap(), plain);
}

#[test]
fn corrupt_blob_is_an_error() {
    use_built_binary();
    let gz = gzip(&b"hello hello hello ".repeat(500));
    let codec = by_id("precomp").unwrap();
    let mut packed = pack(codec, &gz, Level::Medium).unwrap();
    let n = packed.len();
    packed[n - 3] ^= 0xFF;
    if let Ok(back) = unpack(codec, &packed, gz.len()) {
        assert_ne!(back, gz, "corruption went unnoticed");
    }
}
