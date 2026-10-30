use smartcomp::codecs::{by_spec, pack, unpack, Level};

const IDS: &[&str] = &["store", "lzma2", "lzma2-bcj", "ppmd7", "zstd", "brotli", "uharc-mx", "uharc-m3", "ppmd7:16", "ppmd7:32", "lzma2:pcm", "lzma2:rec", "nzcm"];

fn samples() -> Vec<(&'static str, Vec<u8>)> {
    let text: Vec<u8> = "the quick brown fox jumps over the lazy dog. ".repeat(5000).into_bytes();
    // Deterministic pseudo-random bytes (xorshift), plus x86-looking CALL opcodes for the BCJ filter.
    let mut x = 0x2545F4914F6CDD1Du64;
    let random: Vec<u8> = (0..100_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let mut code = Vec::new();
    for i in 0..20_000u32 {
        code.extend_from_slice(&[0xE8]);
        code.extend_from_slice(&(i * 7).to_le_bytes());
    }
    vec![("text", text), ("random", random), ("zeros", vec![0; 300_000]), ("code", code), ("one", vec![42]), ("tiny", b"ab".to_vec())]
}

#[test]
fn every_codec_roundtrips_at_every_level() {
    for id in IDS {
        let codec = by_spec(id).unwrap();
        for level in [Level::Medium, Level::Ultra, Level::HighUltra] {
            for (name, data) in samples() {
                let packed = pack(codec, &data, level).unwrap_or_else(|e| panic!("{id}/{name}: {e}"));
                let back = unpack(codec, &packed, data.len()).unwrap_or_else(|e| panic!("{id}/{name}: {e}"));
                assert!(back == data, "{id} {level:?} {name}: round-trip mismatch");
            }
        }
    }
}

#[test]
fn empty_input_roundtrips() {
    for id in IDS {
        let codec = by_spec(id).unwrap();
        let packed = pack(codec, &[], Level::Medium).unwrap();
        assert!(unpack(codec, &packed, 0).unwrap().is_empty(), "{id}");
    }
}

#[test]
fn corrupted_data_is_an_error_not_a_panic() {
    for id in ["lzma2", "ppmd7", "zstd", "brotli"] {
        let codec = by_spec(id).unwrap();
        let data = "hello world ".repeat(2000).into_bytes();
        let mut packed = pack(codec, &data, Level::Medium).unwrap();
        let mid = packed.len() / 2;
        for b in &mut packed[mid..mid + 8] {
            *b ^= 0xA5;
        }
        // Either the codec reports an error or the bytes differ; it must never hand back the original.
        if let Ok(back) = unpack(codec, &packed, data.len()) {
            assert!(back != data, "{id}: corruption went unnoticed");
        }
    }
}

#[test]
fn compressors_actually_compress_text() {
    let data = "lorem ipsum dolor sit amet ".repeat(4000).into_bytes();
    for id in ["lzma2", "ppmd7", "zstd", "brotli"] {
        let packed = pack(by_spec(id).unwrap(), &data, Level::Ultra).unwrap();
        assert!(packed.len() * 20 < data.len(), "{id}: {} -> {}", data.len(), packed.len());
    }
}
