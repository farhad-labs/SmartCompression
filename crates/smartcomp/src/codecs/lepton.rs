use std::io::Cursor;

use anyhow::{anyhow, Result};
use lepton_jpeg::{decode_lepton, encode_lepton_verify, EnabledFeatures, DEFAULT_THREAD_POOL};

use super::{Codec, Level};

/// Lossless JPEG recompression (Lepton, ~20% smaller). JPEG only.
pub struct Lepton;

impl Codec for Lepton {
    fn id(&self) -> &'static str {
        "lepton"
    }

    fn version(&self) -> &'static str {
        "0.5.3"
    }

    fn splittable(&self) -> bool {
        false
    }

    fn compress(&self, block: &[u8], _level: Level) -> Result<Vec<u8>> {
        // The _verify variant decodes the result again and compares it with the input.
        let features = EnabledFeatures::compat_lepton_vector_write();
        let (out, _) = encode_lepton_verify(block, &features, &DEFAULT_THREAD_POOL).map_err(|e| anyhow!("lepton: {e}"))?;
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        let features = EnabledFeatures::compat_lepton_vector_read();
        let mut out = Vec::with_capacity(original_len);
        decode_lepton(&mut Cursor::new(blob), &mut out, &features, &DEFAULT_THREAD_POOL).map_err(|e| anyhow!("lepton: {e}"))?;
        Ok(out)
    }
}
