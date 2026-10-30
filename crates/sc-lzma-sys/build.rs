use std::path::PathBuf;

// 7-Zip's plain-C LZMA SDK, built single-threaded (Z7_ST): parallelism comes from
// compressing independent files/blocks, not from inside one LZMA2 stream.
const FILES: &[&str] = &[
    "7zCrc.c", "7zCrcOpt.c", "Alloc.c", "Bra.c", "Bra86.c", "CpuArch.c", "Delta.c", "LzFind.c", "LzFindOpt.c",
    "Lzma2Dec.c", "Lzma2Enc.c", "LzmaDec.c", "LzmaEnc.c", "Ppmd7.c", "Ppmd7Dec.c", "Ppmd7Enc.c",
];

fn main() {
    let c_dir = PathBuf::from("../../third_party/7zip/C");
    let mut b = cc::Build::new();
    b.include(&c_dir).define("Z7_ST", None).opt_level(3).warnings(false);
    for f in FILES {
        b.file(c_dir.join(f));
        println!("cargo:rerun-if-changed={}", c_dir.join(f).display());
    }
    b.file("shim.c");
    println!("cargo:rerun-if-changed=shim.c");
    b.compile("sc_lzma");
}
