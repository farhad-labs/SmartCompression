//! Minimal bindings to the brotli one-shot API.

use std::os::raw::{c_int, c_void};

pub const BROTLI_MODE_GENERIC: c_int = 0;
pub const BROTLI_MODE_TEXT: c_int = 1;
pub const BROTLI_DECODER_RESULT_SUCCESS: c_int = 1;

extern "C" {
    pub fn BrotliEncoderMaxCompressedSize(input_size: usize) -> usize;
    pub fn BrotliEncoderCompress(
        quality: c_int,
        lgwin: c_int,
        mode: c_int,
        input_size: usize,
        input: *const u8,
        encoded_size: *mut usize,
        encoded: *mut u8,
    ) -> c_int;
    pub fn BrotliDecoderDecompress(encoded_size: usize, encoded: *const u8, decoded_size: *mut usize, decoded: *mut u8) -> c_int;
}

#[allow(dead_code)]
type _Unused = *mut c_void;
