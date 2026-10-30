use anyhow::{ensure, Result};

use super::{Codec, Level};

/// PPMd var.H with the 7z range coder (7-Zip C SDK). Strong on text and markup.
pub struct Ppmd7 {
    pub order: u32,
    pub spec: &'static str,
}

fn mem_cap(level: Level) -> u32 {
    if level.is_ultra() {
        512 << 20
    } else {
        64 << 20
    }
}

impl Codec for Ppmd7 {
    fn id(&self) -> &'static str {
        "ppmd7"
    }

    fn version(&self) -> &'static str {
        "7zip-26.05"
    }

    fn spec(&self) -> &'static str {
        self.spec
    }

    fn params(&self, level: Level) -> String {
        format!("order={} mem={}M", self.order, mem_cap(level) >> 20)
    }

    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>> {
        let order = self.order;
        // The model needs roughly 12-16 bytes per input byte before it starts recycling.
        let mem = (block.len() as u64 * 16).clamp(1 << 20, mem_cap(level) as u64) as u32;

        // Blob layout: [order u8][mem u32 LE][range-coder stream].
        let mut out = vec![0u8; 5 + block.len() + block.len() / 4 + 1024];
        let mut out_len = out.len() - 5;
        let rc = unsafe { sc_lzma_sys::sc_ppmd7_encode(block.as_ptr(), block.len(), out[5..].as_mut_ptr(), &mut out_len, order, mem) };
        ensure!(rc == 0, "PPMd encoder failed (code {rc})");
        out[0] = order as u8;
        out[1..5].copy_from_slice(&mem.to_le_bytes());
        out.truncate(5 + out_len);
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        ensure!(blob.len() >= 5, "PPMd blob too short");
        let order = blob[0] as u32;
        let mem = u32::from_le_bytes(blob[1..5].try_into().unwrap());
        ensure!((2..=64).contains(&order) && mem >= 1 << 11, "invalid PPMd parameters");
        let mut out = vec![0u8; original_len];
        let rc = unsafe { sc_lzma_sys::sc_ppmd7_decode(out.as_mut_ptr(), out.len(), blob[5..].as_ptr(), blob.len() - 5, order, mem) };
        ensure!(rc == 0, "PPMd data error (code {rc})");
        Ok(out)
    }
}
