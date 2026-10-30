use std::path::{Path, PathBuf};

// Mirrors the source lists of precomp-cpp's CMakeLists.txt, except that its bundled brotli 1.0.7
// is left out: sc-brotli-sys provides brotli 1.2.0, and two copies would collide at link time.
const GIF: &[&str] = &["gifalloc", "gif_err", "dgif_lib_gcc", "egif_lib_gcc"];
const BZIP: &[&str] = &["bzlib", "blocksort", "crctable", "compress", "decompress", "huffman", "randtable"];
const ZLIB: &[&str] = &["adler32", "crc32", "zutil", "trees", "inftrees", "inffast", "inflate", "deflate"];
const LZMA_COMMON: &[&str] = &[
    "tuklib_physmem", "tuklib_cpucores", "common", "block_util", "easy_preset", "filter_common", "hardware_physmem", "index",
    "stream_flags_common", "vli_size", "alone_encoder", "block_buffer_encoder", "block_encoder", "block_header_encoder",
    "easy_buffer_encoder", "easy_encoder", "easy_encoder_memusage", "filter_buffer_encoder", "filter_encoder",
    "filter_flags_encoder", "index_encoder", "stream_buffer_encoder", "stream_encoder", "stream_flags_encoder", "vli_encoder",
    "hardware_cputhreads", "outqueue", "stream_encoder_mt", "alone_decoder", "auto_decoder", "block_buffer_decoder",
    "block_decoder", "block_header_decoder", "easy_decoder_memusage", "filter_buffer_decoder", "filter_decoder",
    "filter_flags_decoder", "index_decoder", "index_hash", "stream_decoder", "stream_buffer_decoder", "stream_flags_decoder",
    "vli_decoder",
];
const LZMA_CHECK: &[&str] = &["check", "crc32_table", "crc32_fast", "crc64_table", "crc64_fast", "sha256"];
const LZMA_LZ: &[&str] = &["lz_encoder", "lz_encoder_mf", "lz_decoder"];
const LZMA_LZMA: &[&str] = &[
    "lzma_encoder", "lzma_encoder_presets", "lzma_encoder_optimum_fast", "lzma_encoder_optimum_normal", "fastpos_table",
    "lzma_decoder", "lzma2_encoder", "lzma2_decoder",
];
const LZMA_SIMPLE: &[&str] = &["simple_coder", "simple_encoder", "simple_decoder", "x86", "powerpc", "ia64", "arm", "armthumb", "sparc"];
const DELTA: &[&str] = &["delta_common", "delta_encoder", "delta_decoder"];
const PACKJPG: &[&str] = &["aricoder", "bitops", "packjpg"];
const PACKMP3: &[&str] = &["huffmp3", "packmp3"];
const BRUNSLI_COMMON: &[&str] = &["ans_params", "constants", "context", "huffman_tree", "lehmer_code", "platform", "predict", "quant_matrix"];
const BRUNSLI_DEC: &[&str] = &[
    "ans_decode", "bit_reader", "brunsli_decode", "context_map_decode", "histogram_decode", "huffman_decode", "huffman_table",
    "jpeg_data_writer",
];
const BRUNSLI_ENC: &[&str] = &[
    "ans_encode", "brunsli_encode", "context_map_encode", "histogram_encode", "jpeg_data_reader", "jpeg_huffman_decode", "write_bits",
];
const PREFLATE: &[&str] = &[
    "block_decoder", "block_reencoder", "block_trees", "complevel_estimator", "constants", "decoder", "hash_chain", "info",
    "parameter_estimator", "parser_config", "predictor_state", "reencoder", "seq_chain", "statistical_codec", "statistical_model",
    "token", "token_predictor", "tree_predictor",
];
const PREFLATE_SUPPORT: &[&str] = &[
    "arithmetic_coder", "array_helper", "bit_helper", "bitstream", "const_division", "filestream", "huffman_decoder",
    "huffman_encoder", "huffman_helper", "memstream", "outputcachestream", "task_pool",
];

fn files(dir: PathBuf, prefix: &str, stems: &[&str], ext: &str) -> Vec<PathBuf> {
    stems
        .iter()
        .map(|s| {
            let p = dir.join(format!("{prefix}{s}.{ext}"));
            assert!(p.is_file(), "missing precomp source: {}", p.display());
            p
        })
        .collect()
}

fn main() {
    let root = PathBuf::from("../../third_party/precomp-cpp");
    let c = root.join("contrib");
    let lz = c.join("liblzma");

    let includes: Vec<PathBuf> = [
        root.clone(), lz.clone(), lz.join("api"), lz.join("check"), lz.join("common"), lz.join("delta"), lz.join("lz"),
        lz.join("lzma"), lz.join("rangecoder"), lz.join("simple"), c.join("preflate"), c.join("brunsli/c/common"),
        c.join("brunsli/c/dec"), c.join("brunsli/c/enc"), c.join("brunsli/c/include"),
        // Headers of brotli 1.0.7 are what brunsli was written against; the 1.2.0 library implements them.
        PathBuf::from("../../third_party/brotli/c/include"),
    ]
    .into();

    let mut c_files: Vec<PathBuf> = Vec::new();
    c_files.extend(files(c.join("giflib"), "", GIF, "c"));
    c_files.extend(files(c.join("bzip2"), "", BZIP, "c"));
    c_files.extend(files(c.join("zlib"), "", ZLIB, "c"));
    c_files.extend(files(lz.join("common"), "", LZMA_COMMON, "c"));
    c_files.extend(files(lz.join("check"), "", LZMA_CHECK, "c"));
    c_files.extend(files(lz.join("lz"), "", LZMA_LZ, "c"));
    c_files.extend(files(lz.join("lzma"), "", LZMA_LZMA, "c"));
    c_files.extend(files(lz.join("rangecoder"), "", &["price_table"], "c"));
    c_files.extend(files(lz.join("delta"), "", DELTA, "c"));
    c_files.extend(files(lz.join("simple"), "", LZMA_SIMPLE, "c"));

    let mut cpp_files: Vec<PathBuf> = Vec::new();
    cpp_files.extend(files(c.join("packjpg"), "", PACKJPG, "cpp"));
    cpp_files.extend(files(c.join("packmp3"), "", PACKMP3, "cpp"));
    cpp_files.extend(files(lz.clone(), "", &["compress_easy_mt"], "cpp"));
    cpp_files.extend(files(c.join("brunsli/c/common"), "", BRUNSLI_COMMON, "cc"));
    cpp_files.extend(files(c.join("brunsli/c/dec"), "", BRUNSLI_DEC, "cc"));
    cpp_files.extend(files(c.join("brunsli/c/enc"), "", BRUNSLI_ENC, "cc"));
    cpp_files.extend(files(c.join("preflate"), "preflate_", PREFLATE, "cpp"));
    cpp_files.extend(files(c.join("preflate/support"), "", PREFLATE_SUPPORT, "cpp"));
    cpp_files.push(PathBuf::from("shim.cpp"));

    let msvc = cfg!(target_env = "msvc");
    let define_common = |b: &mut cc::Build| {
        for i in &includes {
            b.include(i);
        }
        b.define("HAVE_BOOL", None).define("LZMA_API_STATIC", None).define("BUILD_LIB", None).define("PRECOMPDLL", None).opt_level(2).warnings(false);
        if cfg!(target_pointer_width = "64") {
            b.define("BIT64", None);
        }
        // Without this macOS takes precomp's Windows code path (it only knows __unix).
        if cfg!(unix) {
            b.define("__unix", None);
        }
        if msvc {
            b.define("_UNICODE", None).define("UNICODE", None).include(root.join("vs2015")).flag("/EHsc");
        }
    };

    let mut cb = cc::Build::new();
    define_common(&mut cb);
    cb.files(&c_files);
    cb.compile("sc_precomp_c");

    let mut xb = cc::Build::new();
    define_common(&mut xb);
    xb.cpp(true).std("c++14").files(&cpp_files);
    xb.compile("sc_precomp_cpp");

    for f in c_files.iter().chain(&cpp_files) {
        println!("cargo:rerun-if-changed={}", Path::new(f).display());
    }
    println!("cargo:rerun-if-changed={}", root.join("precomp.cpp").display());
}
