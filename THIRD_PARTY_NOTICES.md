# Third-party notices

SmartCompression itself is licensed under the terms in [LICENSE](LICENSE)
(PolyForm Noncommercial 1.0.0; commercial use needs permission). The components below are
included unmodified (or, where noted, built with a build-time patch) under `third_party/` and
remain under their **own** licenses, which are not changed by SmartCompression's license.
Each component's license file is kept next to its sources.

| Component | Used for | Upstream / author | License | Location |
|---|---|---|---|---|
| LZMA SDK / 7-Zip `C/` (LZMA2, PPMd7, BCJ, delta) | `lzma2`, `ppmd7` codecs | Igor Pavlov, 7-zip.org | Public domain (LZMA SDK); see `License.txt` | `third_party/7zip/` |
| Zstandard | `zstd` codec | Meta Platforms | BSD-3-Clause (or GPL-2.0, at your choice) | `third_party/zstd/` |
| Brotli | `brotli` codec (and precomp's brunsli) | Google / the Brotli Authors | MIT | `third_party/brotli/` |
| Lepton JPEG (Rust port) | lossless JPEG recompression | Microsoft | Apache-2.0 | `third_party/lepton_jpeg/` |
| packMP3 (Rust port) | lossless MP3 recompression | Matthias Stirner (packMP3); Rust port | **LGPL-3.0-or-later** | `third_party/packmp3/` |
| Precomp | PNG/ZIP/GZ/PDF stream recompression | Christian Schneider and contributors | Apache-2.0 (+ bundled libraries below) | `third_party/precomp-cpp/` |
| - preflate, brunsli, bzip2, mingw_std_threads | bundled by Precomp | their authors | see each `contrib/*/LICENSE` | `third_party/precomp-cpp/contrib/` |
| - zlib, giflib, liblzma (XZ Utils), packJPG | bundled by Precomp | their authors | zlib / MIT / public domain / see source headers | `third_party/precomp-cpp/contrib/` |
| nanozip-re (`nz_cm` context-mixing coder) | `nzcm` codec (`highultra`) | YadeWira; reimplementation of NanoZip by Sami Runsas | MIT (see its README) | `third_party/nanozip-re/` |
| UHARC 0.6b engine (`uharc-lifted`) | `uharc-mx` codec | UHARC format and design (c) 1997-2005 Uwe Herklotz; Rust implementation by farhad-labs | see note below | `third_party/uharc/` |

`nz_cm.cpp` is patched at build time (one `__builtin_clz` line, in a copy under `OUT_DIR`) so it
compiles with MSVC; the source under `third_party/` is untouched.

## LGPL-3.0 note (packMP3)

`packmp3` is linked statically. Its complete source is in `third_party/packmp3/` and the whole
program is distributed as source, so you can modify it and rebuild (`cargo build --release`) as
the LGPL requires. SmartCompression's non-commercial restriction applies only to SmartCompression's
own code, **not** to packMP3.

## UHARC note

UHARC was created by Uwe Herklotz; the original program is "freeware for non-commercial use"
(see `third_party/uharc/UHARC.DOC`). The codec is credited to him here. If you are the rights
holder and want something changed, please open an issue.

## Rust crates (fetched by Cargo at build time)

aes-gcm, anyhow, argon2, blake3, clap, indicatif, rand, rayon, rpassword, serde, serde_json,
tempfile, toml, walkdir (and flate2 for tests), plus their transitive dependencies; and `cc` for build
scripts. They are licensed MIT and/or Apache-2.0 (blake3 also CC0-1.0); see `Cargo.lock` for exact
versions and each crate's page on crates.io for the license text.
