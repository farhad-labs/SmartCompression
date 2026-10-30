use std::ffi::CStr;
use std::os::raw::c_void;

use anyhow::{bail, Result};
use sc_zstd_sys as z;

use super::{Codec, Level};

pub struct Zstd;

fn check(code: usize) -> Result<usize> {
    if unsafe { z::ZSTD_isError(code) } != 0 {
        let name = unsafe { CStr::from_ptr(z::ZSTD_getErrorName(code)) };
        bail!("zstd: {}", name.to_string_lossy());
    }
    Ok(code)
}

impl Codec for Zstd {
    fn id(&self) -> &'static str {
        "zstd"
    }

    fn version(&self) -> &'static str {
        "1.6.0"
    }

    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>> {
        let lvl = if level.is_ultra() { 22 } else { 12 };
        let mut out = vec![0u8; unsafe { z::ZSTD_compressBound(block.len()) }];
        unsafe {
            let c = z::ZSTD_createCCtx();
            if c.is_null() {
                bail!("zstd: out of memory");
            }
            let res = (|| {
                check(z::ZSTD_CCtx_setParameter(c, z::ZSTD_C_COMPRESSION_LEVEL, lvl))?;
                check(z::ZSTD_CCtx_setParameter(c, z::ZSTD_C_WINDOW_LOG, 27))?;
                check(z::ZSTD_CCtx_setParameter(c, z::ZSTD_C_ENABLE_LONG_DISTANCE_MATCHING, 1))?;
                check(z::ZSTD_compress2(c, out.as_mut_ptr() as *mut c_void, out.len(), block.as_ptr() as *const c_void, block.len()))
            })();
            z::ZSTD_freeCCtx(c);
            out.truncate(res?);
        }
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; original_len];
        unsafe {
            let d = z::ZSTD_createDCtx();
            if d.is_null() {
                bail!("zstd: out of memory");
            }
            let res = (|| {
                check(z::ZSTD_DCtx_setParameter(d, z::ZSTD_D_WINDOW_LOG_MAX, 27))?;
                check(z::ZSTD_decompressDCtx(d, out.as_mut_ptr() as *mut c_void, out.len(), blob.as_ptr() as *const c_void, blob.len()))
            })();
            z::ZSTD_freeDCtx(d);
            if res? != original_len {
                bail!("zstd: decoded size differs from the manifest");
            }
        }
        Ok(out)
    }
}
