//! Raw bindings to `shim.cpp`; the safe wrapper lives in the `smartcomp` crate.

extern "C" {
    pub fn sc_nzcm_encode(data: *const u8, n: u32, a_bits: i32, b_bits: i32, out: *mut u8, out_cap: u32, out_len: *mut u32) -> i32;
    pub fn sc_nzcm_decode(input: *const u8, in_len: u32, a_bits: i32, b_bits: i32, out: *mut u8, out_n: u32) -> i32;
}
