use anyhow::{bail, ensure, Result};
use sc_brotli_sys as b;

use super::{Codec, Level};

pub struct Brotli;

impl Codec for Brotli {
    fn id(&self) -> &'static str {
        "brotli"
    }

    fn version(&self) -> &'static str {
        "1.2.0"
    }

    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>> {
        let (quality, lgwin) = if level.is_ultra() { (11, 24) } else { (9, 22) };
        let mut out = vec![0u8; unsafe { b::BrotliEncoderMaxCompressedSize(block.len()) }.max(64)];
        let mut out_len = out.len();
        let ok = unsafe {
            b::BrotliEncoderCompress(quality, lgwin, b::BROTLI_MODE_GENERIC, block.len(), block.as_ptr(), &mut out_len, out.as_mut_ptr())
        };
        ensure!(ok == 1, "brotli encoder failed");
        out.truncate(out_len);
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; original_len];
        let mut out_len = original_len;
        let res = unsafe { b::BrotliDecoderDecompress(blob.len(), blob.as_ptr(), &mut out_len, out.as_mut_ptr()) };
        if res != b::BROTLI_DECODER_RESULT_SUCCESS || out_len != original_len {
            bail!("brotli: corrupted data");
        }
        Ok(out)
    }
}
