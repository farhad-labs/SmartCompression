//! Compresses one in-memory block with UHARC's PPM (`-mx`) or ALZ:3 (`-m3`) coder.
//!
//! The original program feeds file data through a file list on disk. Here the block goes
//! through the engine's table path instead, which runs the same coders on a memory buffer.
//! The output is therefore not a `.uha` stream, only something this crate can decode.
//!
//! Blob layout: `[method byte][text flag][coder stream]`.

mod engine;

use engine::Engine;
use uharc_lifted::engine as lifted;

// Engine globals, from uharc06b-rs/src/create.rs.
const G_BUFFER_KB: u32 = 0x100000;
const G_METHOD_CORE: u32 = 0x436f74; // method -> codec core
const G_EXE_CORE: u32 = 0x449037; // "the method's core is the PPM coder"
const G_TEXT_GATE: u32 = 0x43731c;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `-mx`: PPM, the slowest and strongest mode.
    Ppm,
    /// `-m3`: ALZ level 3.
    Alz3,
}

#[derive(Debug)]
pub struct EngineError(pub u32);

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UHARC engine error {}", self.0)
    }
}

impl std::error::Error for EngineError {}

/// Dictionary size in KB: at most `max_kb`, halved while the block fits in half of it.
fn dict_bits(len: usize, max_kb: u32) -> u32 {
    let mut dict = max_kb.clamp(1, 32768);
    while dict >= 2 && (len as u64) < ((dict >> 1) as u64) << 10 {
        dict >>= 1;
    }
    dict.next_power_of_two().trailing_zeros()
}

fn method_byte(mode: Mode, bits: u32) -> u8 {
    match mode {
        Mode::Ppm => 0x18 + bits as u8,
        Mode::Alz3 => 0x28 + 0x20 + bits as u8,
    }
}

fn prepare(eng: &mut Engine) {
    eng.call(lifted::f_40d400, G_BUFFER_KB, 0, 0, 0, &[]);
    eng.call(lifted::f_40e990, 0, 0, 0, 0, &[]);
}

/// The engine tunes its PPM model for text when told the data is text (about 9% smaller
/// on prose). UHARC.EXE decides that by scanning files; this is a simple equivalent.
pub fn looks_like_text(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(64 << 10)];
    let texty = sample.iter().filter(|b| matches!(**b, 9 | 10 | 13 | 32..=126 | 128..=255)).count();
    !sample.is_empty() && texty * 100 >= sample.len() * 95 && !sample.contains(&0)
}

/// Primes the engine the way `compress_data` does once the data stream is known.
fn prime(eng: &mut Engine, method: u8, text_bytes: u32) {
    let core = eng.cpu().rd8(G_METHOD_CORE + method as u32);
    eng.cpu().wr8(G_EXE_CORE, (core == 4) as u32);
    eng.cpu().wr32(G_TEXT_GATE, text_bytes);
}

pub fn compress(mode: Mode, data: &[u8], max_dict_kb: u32) -> Result<Vec<u8>, EngineError> {
    let text = looks_like_text(data);
    let method = method_byte(mode, dict_bits(data.len(), max_dict_kb));
    let mut eng = Engine::new();
    prepare(&mut eng);
    match mode {
        Mode::Ppm => {
            eng.call(lifted::f_416390, 1, 0, 0, 0, &[]);
        }
        Mode::Alz3 => {
            eng.call(lifted::f_40f2a0, 1, 0, 0x63, 0, &[]);
        }
    }
    prime(&mut eng, method, if text { data.len() as u32 } else { 0 });
    let stream = eng.compress(method, data, false).map_err(EngineError)?;
    let mut blob = Vec::with_capacity(2 + stream.len());
    blob.extend_from_slice(&[method, text as u8]);
    blob.extend_from_slice(&stream);
    Ok(blob)
}

pub fn decompress(blob: &[u8], original_len: usize) -> Result<Vec<u8>, EngineError> {
    let [method, text, stream @ ..] = blob else { return Err(EngineError(u32::MAX)) };
    let (method, text) = (*method, *text != 0);
    // Only the methods `compress` can produce are accepted: the engine is machine code
    // translated 1:1, so unknown method bytes would reach code paths nobody has tested.
    if !matches!(method, 0x18..=0x27 | 0x48..=0x57) {
        return Err(EngineError(u32::MAX - 1));
    }
    let mut eng = Engine::new();
    prepare(&mut eng);
    prime(&mut eng, method, if text { original_len as u32 } else { 0 });
    let out = eng.decompress(method, stream, original_len as u32, false).map_err(EngineError)?;
    if out.len() < original_len {
        return Err(EngineError(u32::MAX - 2));
    }
    Ok(out[..original_len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> Vec<u8> {
        "It was the best of times, it was the worst of times, it was the age of wisdom. ".repeat(3000).into_bytes()
    }

    #[test]
    fn ppm_roundtrip() {
        let data = text();
        let blob = compress(Mode::Ppm, &data, 4096).unwrap();
        assert!(blob.len() * 20 < data.len(), "{} -> {}", data.len(), blob.len());
        assert_eq!(decompress(&blob, data.len()).unwrap(), data);
    }

    #[test]
    fn alz3_roundtrip() {
        let data = text();
        let blob = compress(Mode::Alz3, &data, 4096).unwrap();
        assert!(blob.len() * 10 < data.len(), "{} -> {}", data.len(), blob.len());
        assert_eq!(decompress(&blob, data.len()).unwrap(), data);
    }

    #[test]
    fn text_flag_is_part_of_the_blob() {
        let prose = text();
        assert!(looks_like_text(&prose));
        assert_eq!(compress(Mode::Ppm, &prose, 4096).unwrap()[1], 1);
        assert_eq!(compress(Mode::Ppm, &[0u8; 5000], 4096).unwrap()[1], 0);
    }

    #[test]
    fn binary_and_tiny_roundtrip() {
        let mut x = 12345u32;
        let bin: Vec<u8> = (0..70_000)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 24) as u8
            })
            .collect();
        for mode in [Mode::Ppm, Mode::Alz3] {
            for data in [&bin[..], b"x", b"hello"] {
                let blob = compress(mode, data, 32768).unwrap();
                assert_eq!(decompress(&blob, data.len()).unwrap(), data, "{mode:?} len {}", data.len());
            }
        }
    }
}
