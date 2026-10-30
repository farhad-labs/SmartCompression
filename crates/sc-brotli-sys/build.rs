use std::fs;
use std::path::PathBuf;

fn main() {
    let c = PathBuf::from("../../third_party/brotli/c");
    let mut b = cc::Build::new();
    b.include(c.join("include")).opt_level(3).warnings(false);
    for dir in ["common", "dec", "enc"] {
        for e in fs::read_dir(c.join(dir)).expect("brotli source dir") {
            let p = e.unwrap().path();
            // static_init_lazy.cc is only used with BROTLI_STATIC_INIT=LAZY (not the default).
            if p.extension().is_some_and(|x| x == "c") {
                println!("cargo:rerun-if-changed={}", p.display());
                b.file(p);
            }
        }
    }
    b.compile("sc_brotli");
}
