//! Raw binding to `shim.cpp` (precomp-cpp's DLL interface). Not re-entrant and it may call
//! `exit()`: use it only from a dedicated child process, never from the archiver itself.

// Keeps the brotli library linked: precomp's brunsli code resolves its Brotli* calls against it.
extern crate sc_brotli_sys;

use std::os::raw::{c_char, c_int};

extern "C" {
    pub fn sc_precomp_run(mode: c_int, in_file: *const c_char, out_file: *const c_char, msg: *mut c_char) -> c_int;
}
