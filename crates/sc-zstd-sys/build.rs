use std::fs;
use std::path::PathBuf;

fn main() {
    let lib = PathBuf::from("../../third_party/zstd/lib");
    let mut b = cc::Build::new();
    b.include(&lib)
        .include(lib.join("common"))
        .define("ZSTD_DISABLE_ASM", None)
        .define("ZSTD_LEGACY_SUPPORT", "0")
        .opt_level(3)
        .warnings(false);
    for dir in ["common", "compress", "decompress"] {
        for e in fs::read_dir(lib.join(dir)).expect("zstd source dir") {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "c") {
                println!("cargo:rerun-if-changed={}", p.display());
                b.file(p);
            }
        }
    }
    b.compile("sc_zstd");
}
