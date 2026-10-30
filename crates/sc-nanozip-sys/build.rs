use std::path::PathBuf;
use std::{env, fs};

// Builds nanozip-re's context-mixing coder (nz_cm.cpp) unmodified on disk. The one line that
// needs `__builtin_clz` is replaced in a copy, so the source folder stays untouched and MSVC
// (which has no such builtin) can compile it.
const BSR_ORIGINAL: &str = "#define BSR(x) (31u - static_cast<unsigned>(__builtin_clz(static_cast<unsigned>(x))))";
const BSR_PORTABLE: &str = "static inline unsigned sc_bsr(unsigned x) { unsigned r = 0; while (x >>= 1) ++r; return r; }\n#define BSR(x) sc_bsr(static_cast<unsigned>(x))";

fn main() {
    let root = PathBuf::from("../../third_party/nanozip-re");
    let src = root.join("src/nz_cm.cpp");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed=shim.cpp");

    let text = fs::read_to_string(&src).expect("nanozip-re-main/src/nz_cm.cpp");
    assert!(text.contains(BSR_ORIGINAL), "nz_cm.cpp changed: the BSR line to patch was not found");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("nz_cm_portable.cpp");
    fs::write(&out, text.replace(BSR_ORIGINAL, BSR_PORTABLE)).unwrap();

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(root.join("include"))
        .file(&out)
        .file("shim.cpp")
        .opt_level(3)
        .warnings(false)
        .compile("sc_nanozip");
}
