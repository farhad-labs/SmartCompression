use anyhow::{bail, ensure, Result};
use packmp3::app::{analyze_frames, read_mp3, write_mp3};
use packmp3::bitops::{IoSource, IoStream, MODE_READ, MODE_WRITE, TYPE_MEMORY};
use packmp3::pmp::{compress_mp3, uncompress_pmp};
use packmp3::state::PackState;

use super::{Codec, Level};

/// Lossless MP3 recompression (packMP3, ~15-20% smaller). MPEG-1 Layer III only.
pub struct PackMp3;

fn mp3_to_pmp(mp3: &[u8]) -> Result<Vec<u8>> {
    ensure!(mp3.len() < i32::MAX as usize, "file too large for packMP3");
    let mut st = PackState::new();
    let mut src = IoStream::new(&IoSource::Memory(mp3), TYPE_MEMORY, 0, MODE_READ);
    if !read_mp3(&mut st, &mut src) || !analyze_frames(&mut st) {
        bail!("packMP3: {}", st.errormessage);
    }
    let mut out = IoStream::new(&IoSource::Memory(&[]), TYPE_MEMORY, 0, MODE_WRITE);
    if !compress_mp3(&mut st, &mut out) {
        bail!("packMP3: {}", st.errormessage);
    }
    Ok(out.getptr())
}

fn pmp_to_mp3(pmp: &[u8]) -> Result<Vec<u8>> {
    ensure!(pmp.len() < i32::MAX as usize, "file too large for packMP3");
    let mut st = PackState::new();
    let mut src = IoStream::new(&IoSource::Memory(pmp), TYPE_MEMORY, 0, MODE_READ);
    if !uncompress_pmp(&mut st, &mut src) {
        bail!("packMP3: {}", st.errormessage);
    }
    let mut out = IoStream::new(&IoSource::Memory(&[]), TYPE_MEMORY, 0, MODE_WRITE);
    if !write_mp3(&mut st, &mut out) {
        bail!("packMP3: {}", st.errormessage);
    }
    Ok(out.getptr())
}

impl Codec for PackMp3 {
    fn id(&self) -> &'static str {
        "packmp3"
    }

    fn version(&self) -> &'static str {
        "1.0g-rs"
    }

    fn splittable(&self) -> bool {
        false
    }

    fn compress(&self, block: &[u8], _level: Level) -> Result<Vec<u8>> {
        let pmp = mp3_to_pmp(block)?;
        // packMP3 can accept files it cannot reproduce exactly, so prove the round trip.
        ensure!(pmp_to_mp3(&pmp)? == block, "packMP3 round trip differs from the input");
        Ok(pmp)
    }

    fn decompress(&self, blob: &[u8], _original_len: usize) -> Result<Vec<u8>> {
        pmp_to_mp3(blob)
    }
}
