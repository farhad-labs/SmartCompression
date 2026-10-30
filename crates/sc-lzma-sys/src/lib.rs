//! Raw bindings to `shim.c`; the safe wrappers live in the `smartcomp` crate.

extern "C" {
    pub fn sc_lzma2_encode(
        src: *const u8,
        n: usize,
        dst: *mut u8,
        dst_len: *mut usize,
        level: i32,
        dict: u32,
        lc: i32,
        lp: i32,
        pb: i32,
        fb: i32,
        prop_out: *mut u8,
    ) -> i32;
    pub fn sc_lzma2_decode(dst: *mut u8, dst_len: usize, src: *const u8, src_len: usize, prop: u8) -> i32;
    pub fn sc_ppmd7_encode(src: *const u8, n: usize, dst: *mut u8, dst_len: *mut usize, order: u32, mem: u32) -> i32;
    pub fn sc_ppmd7_decode(dst: *mut u8, n: usize, src: *const u8, src_len: usize, order: u32, mem: u32) -> i32;
    pub fn sc_x86_bcj(data: *mut u8, n: usize, encode: i32);
}
