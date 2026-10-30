//! Which codecs make sense for a file. The table encodes BENCHMARK.md: for each kind of
//! data it lists the sensible codecs, best first, and `choose` probes them.

use anyhow::Result;
use rayon::prelude::*;

use crate::codecs::{self, Codec, Level};

/// Coarse content class used by the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Text,
    Executable,
    Jpeg,
    Mp3,
    /// Deflate/zlib inside: PNG, GIF, ZIP family, gzip. Precomp can re-inflate these.
    Deflate,
    /// Already compressed (video, archives, images with their own entropy coding).
    Packed,
    Audio,
    Binary,
}

fn is_texty(data: &[u8]) -> bool {
    sc_uharc::looks_like_text(data)
}

pub fn classify(format: &str, data: &[u8]) -> Class {
    match format {
        "jpeg" | "jpg" => Class::Jpeg,
        "mp3" => Class::Mp3,
        "text" | "html" | "xml" | "json" | "csv" | "pdf" | "svg" | "css" | "js" | "rtf" | "yaml" | "toml" | "ini" | "md" | "sql" => Class::Text,
        "exe" | "dll" | "elf" | "macho" | "macho_fat" | "mz" | "so" | "class" => Class::Executable,
        "wav" | "bmp" | "tiff" | "flac_raw" => Class::Audio,
        "png" | "gif" | "zip" | "gzip" | "gz" | "docx" | "xlsx" | "pptx" | "odf" | "epub" | "apk" | "jar" | "swf" => Class::Deflate,
        "mp4" | "mov" | "3gp" | "m4a" | "mkv" | "webm" | "avi" | "7z" | "rar" | "xz" | "bz2" | "zst" | "webp" | "heic" | "avif" | "ogg" | "opus" | "flac" => Class::Packed,
        // Containers whose payload is unknown: look at the bytes.
        _ if is_texty(data) => Class::Text,
        _ => Class::Binary,
    }
}

/// Codec specs that may be used for `class` at `level`, preferred first. Never empty.
///
/// * Medium: one or two cheap candidates.
/// * Ultra: several strong coders, compared by probing (PPMd, UHARC, LZMA2 with filters).
/// * HighUltra: nanozip's context-mixing coder. It beats everything else on text, executables,
///   audio and binaries, but runs at only ~0.5 MB/s per core, so above SMALL it is the only
///   candidate; small files can afford a full trial against the faster coders.
pub fn candidates(class: Class, size: usize, level: Level) -> Vec<&'static str> {
    const SMALL: usize = 1 << 20;
    if level == Level::Store {
        return vec!["store"];
    }
    let mut c: Vec<&'static str> = match (class, level) {
        (Class::Jpeg, _) => vec!["lepton", "uharc-mx"],
        (Class::Mp3, _) => vec!["packmp3", "uharc-mx"],
        (Class::Deflate, Level::Medium) => vec!["precomp"],
        (Class::Deflate, _) => vec!["precomp", "lzma2"],
        (Class::Packed, _) => vec!["store", "lzma2"],

        (Class::Text | Class::Executable | Class::Audio | Class::Binary, Level::HighUltra) if size > SMALL => vec!["nzcm"],
        (Class::Text, Level::HighUltra) => vec!["nzcm", "uharc-mx", "ppmd7:16", "ppmd7:8", "lzma2"],
        (Class::Executable, Level::HighUltra) => vec!["nzcm", "lzma2-bcj", "uharc-mx"],
        (Class::Audio, Level::HighUltra) => vec!["nzcm", "lzma2:pcm", "uharc-mx"],
        (Class::Binary, Level::HighUltra) => vec!["nzcm", "lzma2", "uharc-mx", "ppmd7:16"],

        (Class::Text, Level::Ultra) => vec!["ppmd7:16", "uharc-mx", "ppmd7:32", "ppmd7:8", "lzma2"],
        (Class::Executable, Level::Ultra) => vec!["lzma2-bcj", "uharc-mx", "ppmd7:16"],
        (Class::Audio, Level::Ultra) => vec!["lzma2:pcm", "lzma2", "uharc-mx"],
        (Class::Binary, Level::Ultra) => vec!["lzma2", "lzma2:pcm", "lzma2:rec", "ppmd7:16", "uharc-mx"],

        (Class::Text, _) => vec!["ppmd7:8", "lzma2"],
        (Class::Executable, _) => vec!["lzma2-bcj"],
        (Class::Audio, _) => vec!["lzma2:pcm", "lzma2"],
        (Class::Binary, _) => vec!["lzma2", "lzma2:pcm"],
    };
    // Keep the slow PPM-style coders off huge medium-level inputs.
    if level == Level::Medium && size > (256 << 20) {
        c.retain(|id| !matches!(*id, "ppmd7:8" | "ppmd7:16" | "ppmd7:32" | "uharc-mx"));
        if c.is_empty() {
            c.push("lzma2");
        }
    }
    c
}

/// Files up to this size are compressed completely by every candidate; larger ones are probed on a sample.
const FULL_TRIAL: usize = 1 << 20;
const SAMPLE: usize = 1 << 20;
/// An earlier (preferred) candidate wins unless a later one is more than this much smaller.
const TOLERANCE: f64 = 1.003;
/// At Ultra, the two best sample candidates of files up to this size are both run on the whole
/// file: a 1 MiB sample is a poor predictor for executables and other long-range data.
const VERIFY_MAX: usize = 32 << 20;

pub struct Choice {
    pub codec: &'static dyn Codec,
    /// Set when the probe already compressed the whole file with the winner.
    pub packed: Option<Vec<u8>>,
    /// (spec, size) of every candidate that was tried; empty when no probe ran.
    pub probed: Vec<(&'static str, usize)>,
}

fn sample_of(data: &[u8]) -> Vec<u8> {
    // Start, middle and end: a file's head is often unrepresentative (headers, indexes).
    let part = SAMPLE / 3;
    let mid = data.len() / 2 - part / 2;
    [&data[..part], &data[mid..mid + part], &data[data.len() - part..]].concat()
}

pub fn choose(class: Class, data: &[u8], level: Level) -> Result<Choice> {
    let specs = candidates(class, data.len(), level);
    let codecs: Vec<&'static dyn Codec> = specs.iter().map(|s| codecs::by_spec(s)).collect::<Result<_>>()?;
    // Transforms such as lepton and packMP3 need the whole file, so a sample can't be tested.
    if codecs.len() == 1 || data.is_empty() || codecs.iter().any(|c| !c.splittable()) {
        return Ok(Choice { codec: codecs[0], packed: None, probed: Vec::new() });
    }
    let full = data.len() <= FULL_TRIAL;
    let sample;
    let target: &[u8] = if full {
        data
    } else {
        sample = sample_of(data);
        &sample
    };
    let tried: Vec<Option<Vec<u8>>> = codecs.par_iter().map(|c| codecs::pack(*c, target, level).ok()).collect();
    let best = tried.iter().flatten().map(|p| p.len()).min();
    let Some(best) = best else {
        return Ok(Choice { codec: codecs[0], packed: None, probed: Vec::new() });
    };
    let limit = best as f64 * TOLERANCE;
    let win = tried.iter().position(|p| p.as_ref().is_some_and(|p| p.len() as f64 <= limit)).unwrap();
    let probed = specs.iter().zip(&tried).filter_map(|(s, p)| p.as_ref().map(|p| (*s, p.len()))).collect();
    if full {
        let packed = tried.into_iter().nth(win).flatten();
        return Ok(Choice { codec: codecs[win], packed, probed });
    }
    if level == Level::Ultra && data.len() <= VERIFY_MAX {
        // Runner-up = best of the others; both compress the whole file in parallel.
        let mut order: Vec<usize> = (0..codecs.len()).filter(|i| tried[*i].is_some()).collect();
        order.sort_by_key(|i| (tried[*i].as_ref().unwrap().len(), *i));
        order.truncate(2);
        if order.len() == 2 {
            let mut full: Vec<(usize, Option<Vec<u8>>)> =
                order.par_iter().map(|i| (*i, codecs::pack(codecs[*i], data, level).ok())).collect();
            full.retain(|(_, p)| p.is_some());
            if let Some((i, p)) = full.into_iter().min_by_key(|(i, p)| (p.as_ref().unwrap().len(), *i)) {
                return Ok(Choice { codec: codecs[i], packed: p, probed });
            }
        }
    }
    Ok(Choice { codec: codecs[win], packed: None, probed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_follow_the_format() {
        assert_eq!(classify("jpeg", b""), Class::Jpeg);
        assert_eq!(classify("dll", b""), Class::Executable);
        assert_eq!(classify("mp4", b""), Class::Packed);
        assert_eq!(classify("png", b""), Class::Deflate);
        assert_eq!(classify("docx", b""), Class::Deflate);
        assert_eq!(classify("tar", b"plain ascii text here"), Class::Text);
        assert_eq!(classify("unknown", &[0, 1, 2, 3]), Class::Binary);
    }

    #[test]
    fn jpeg_never_goes_to_a_text_coder() {
        let c = candidates(Class::Jpeg, 1000, Level::Ultra);
        assert!(!c.contains(&"ppmd7") && !c.contains(&"nzcm") && c[0] == "lepton");
    }

    #[test]
    fn probe_picks_a_text_coder_for_text_and_never_the_worse_one() {
        let text = "the quick brown fox jumps over the lazy dog. ".repeat(10_000).into_bytes();
        let c = choose(Class::Text, &text, Level::Ultra).unwrap();
        assert!(!c.probed.is_empty());
        let best = c.probed.iter().map(|(_, n)| *n).min().unwrap();
        let chosen = c.probed.iter().find(|(s, _)| *s == c.codec.spec()).unwrap().1;
        assert!(chosen as f64 <= best as f64 * TOLERANCE + 1.0);
    }

    #[test]
    fn small_files_reuse_the_trial_output() {
        let c = choose(Class::Text, b"hello hello hello hello hello hello", Level::Medium).unwrap();
        let packed = c.packed.expect("whole-file trial result");
        assert_eq!(codecs::unpack(c.codec, &packed, 35).unwrap(), b"hello hello hello hello hello hello");
    }

    #[test]
    fn highultra_adds_the_cm_coder_and_ultra_does_not() {
        for class in [Class::Text, Class::Executable, Class::Audio, Class::Binary] {
            assert!(candidates(class, 5 << 20, Level::HighUltra).contains(&"nzcm"), "{class:?}");
            assert!(!candidates(class, 5 << 20, Level::Ultra).contains(&"nzcm"), "{class:?}");
            assert!(!candidates(class, 5 << 20, Level::Medium).contains(&"nzcm"), "{class:?}");
        }
        // Big files get the CM coder alone; small ones are trialled against faster coders.
        assert_eq!(candidates(Class::Text, 5 << 20, Level::HighUltra), vec!["nzcm"]);
        assert!(candidates(Class::Text, 1000, Level::HighUltra).len() > 1);
        // Transforms keep their own coders at every level.
        assert_eq!(candidates(Class::Jpeg, 1000, Level::HighUltra)[0], "lepton");
    }

    #[test]
    fn store_level_only_stores() {
        assert_eq!(candidates(Class::Text, 10, Level::Store), vec!["store"]);
    }
}
