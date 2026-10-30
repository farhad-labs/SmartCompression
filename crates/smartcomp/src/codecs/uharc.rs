use anyhow::{anyhow, Result};
use sc_uharc::Mode;

use super::{Codec, Level};

/// UHARC 0.6b through the lifted engine: `-mx` (PPM) is the default fallback codec,
/// `-m3` (ALZ:3) is the dictionary coder.
pub struct Uharc {
    pub mode: Mode,
}

impl Codec for Uharc {
    fn id(&self) -> &'static str {
        match self.mode {
            Mode::Ppm => "uharc-mx",
            Mode::Alz3 => "uharc-m3",
        }
    }

    fn version(&self) -> &'static str {
        "0.6b"
    }

    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>> {
        let dict_kb = if level.is_ultra() { 32768 } else { 4096 };
        sc_uharc::compress(self.mode, block, dict_kb).map_err(|e| anyhow!("{e}"))
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        sc_uharc::decompress(blob, original_len).map_err(|e| anyhow!("{e}"))
    }
}
