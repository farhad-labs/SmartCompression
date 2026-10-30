//! Minimal bindings to the zstd advanced one-shot API.

use std::os::raw::{c_char, c_int, c_uint, c_void};

pub const ZSTD_C_COMPRESSION_LEVEL: c_int = 100;
pub const ZSTD_C_WINDOW_LOG: c_int = 101;
pub const ZSTD_C_ENABLE_LONG_DISTANCE_MATCHING: c_int = 160;
pub const ZSTD_C_CHECKSUM_FLAG: c_int = 201;

pub enum ZstdCCtx {}
pub enum ZstdDCtx {}

extern "C" {
    pub fn ZSTD_compressBound(src_size: usize) -> usize;
    pub fn ZSTD_createCCtx() -> *mut ZstdCCtx;
    pub fn ZSTD_freeCCtx(c: *mut ZstdCCtx) -> usize;
    pub fn ZSTD_CCtx_setParameter(c: *mut ZstdCCtx, param: c_int, value: c_int) -> usize;
    pub fn ZSTD_compress2(c: *mut ZstdCCtx, dst: *mut c_void, dst_cap: usize, src: *const c_void, src_size: usize) -> usize;
    pub fn ZSTD_createDCtx() -> *mut ZstdDCtx;
    pub fn ZSTD_freeDCtx(d: *mut ZstdDCtx) -> usize;
    pub fn ZSTD_DCtx_setParameter(d: *mut ZstdDCtx, param: c_int, value: c_int) -> usize;
    pub fn ZSTD_decompressDCtx(d: *mut ZstdDCtx, dst: *mut c_void, dst_cap: usize, src: *const c_void, src_size: usize) -> usize;
    pub fn ZSTD_isError(code: usize) -> c_uint;
    pub fn ZSTD_getErrorName(code: usize) -> *const c_char;
}

/// `ZSTD_d_windowLogMax`: needed to decode frames made with a window above the 27-bit default.
pub const ZSTD_D_WINDOW_LOG_MAX: c_int = 100;
