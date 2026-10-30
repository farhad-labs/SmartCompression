use anyhow::{bail, ensure, Result};

use super::{Codec, Level};

/// LZMA2 from the 7-Zip C SDK. With `bcj`, x86 call/jump targets are made absolute
/// first, which helps executables and DLLs.
pub struct Lzma2 {
    pub bcj: bool,
    /// Literal/position bits; -1 keeps the library default (lc3 lp0 pb2).
    pub lc: i32,
    pub lp: i32,
    pub pb: i32,
    pub spec: &'static str,
}

impl Codec for Lzma2 {
    fn id(&self) -> &'static str {
        if self.bcj {
            "lzma2-bcj"
        } else {
            "lzma2"
        }
    }

    fn version(&self) -> &'static str {
        "7zip-26.05"
    }

    fn spec(&self) -> &'static str {
        self.spec
    }

    fn params(&self, level: Level) -> String {
        let (lvl, dict) = if level.is_ultra() { (9, 64) } else { (6, 16) };
        format!("level={lvl} dict={dict}M lc={} lp={} pb={}", self.lc.max(0), self.lp.max(0), if self.pb < 0 { 2 } else { self.pb })
    }

    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>> {
        let (lvl, dict, fb) = match level {
            Level::Ultra | Level::HighUltra => (9, 64u32 << 20, 273),
            _ => (6, 16u32 << 20, 0),
        };
        let dict = dict.min((block.len() as u32).max(1 << 12));
        let filtered;
        let input: &[u8] = if self.bcj {
            let mut copy = block.to_vec();
            unsafe { sc_lzma_sys::sc_x86_bcj(copy.as_mut_ptr(), copy.len(), 1) };
            filtered = copy;
            &filtered
        } else {
            block
        };

        // Blob layout: [prop byte][raw LZMA2 stream].
        let mut out = vec![0u8; 1 + input.len() + input.len() / 3 + 256];
        let mut out_len = out.len() - 1;
        let mut prop = 0u8;
        let rc = unsafe {
            sc_lzma_sys::sc_lzma2_encode(input.as_ptr(), input.len(), out[1..].as_mut_ptr(), &mut out_len, lvl, dict, self.lc, self.lp, self.pb, fb, &mut prop)
        };
        ensure!(rc == 0, "LZMA2 encoder failed (code {rc})");
        out[0] = prop;
        out.truncate(1 + out_len);
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        let Some((&prop, stream)) = blob.split_first() else { bail!("empty LZMA2 blob") };
        let mut out = vec![0u8; original_len];
        let rc = unsafe { sc_lzma_sys::sc_lzma2_decode(out.as_mut_ptr(), out.len(), stream.as_ptr(), stream.len(), prop) };
        ensure!(rc == 0, "LZMA2 data error (code {rc})");
        if self.bcj {
            unsafe { sc_lzma_sys::sc_x86_bcj(out.as_mut_ptr(), out.len(), 0) };
        }
        Ok(out)
    }
}
