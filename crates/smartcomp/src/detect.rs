//! Format detection by magic bytes (never by extension).
//!
//! Signatures come from the format table (offset, bytes, optional mask, priority).
//! The best match (highest priority, then longest) names a *family*; refiners then
//! inspect deeper headers for ambiguous families:
//!   zip      -> docx | xlsx | pptx | odf | epub | apk | jar | zip   (central directory entry names)
//!   riff     -> wav | avi | webp                                    (form type at offset 8)
//!   ebml     -> mkv | webm                                          (DocType)
//!   ftyp     -> mp4 | mov | 3gp | m4a | heic | avif                 (major brand)
//!   cafebabe -> macho_fat | class                                   (Mach-O fat vs Java class)
//!   mp3      -> validated frame header (after skipping an ID3v2 tag)
//!   exe      -> PE signature via e_lfanew, then exe | dll (COFF characteristics), otherwise "mz"
//!   bmp/ico  -> header sanity checks (2-byte signatures are weak)
//! Nothing matched: text heuristic -> "text", else "unknown"; zero-byte file -> "empty".

use std::fs::File;
use std::sync::OnceLock;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, Result};

use serde::Deserialize;

const FORMATS: &str = include_str!("formats.toml");

#[derive(Deserialize)]
struct FormatsFile {
    format: Vec<FormatDef>,
}

#[derive(Deserialize)]
struct FormatDef {
    name: String,
    #[serde(default)]
    priority: i32,
    magic: Vec<Magic>,
}

#[derive(Deserialize)]
struct Magic {
    offset: usize,
    bytes: String,
    mask: Option<String>,
    priority: Option<i32>,
}

fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !cleaned.is_ascii() || cleaned.len() % 2 != 0 {
        bail!("invalid hex string '{s}'");
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|_| anyhow::anyhow!("invalid hex string '{s}'")))
        .collect()
}

const HEAD: usize = 64 * 1024;

#[derive(Debug, Clone)]
struct Signature {
    name: String,
    offset: usize,
    bytes: Vec<u8>,
    mask: Vec<u8>,
    priority: i32,
}

impl Signature {
    fn matches(&self, head: &[u8]) -> bool {
        let end = self.offset + self.bytes.len();
        if head.len() < end {
            return false;
        }
        (0..self.bytes.len()).all(|i| (head[self.offset + i] & self.mask[i]) == (self.bytes[i] & self.mask[i]))
    }
}

pub struct Detector {
    sigs: Vec<Signature>,
}

impl Detector {
    pub fn new() -> Result<Self> {
        let table: FormatsFile = toml::from_str(FORMATS)?;
        let mut sigs = Vec::new();
        for f in &table.format {
            for m in &f.magic {
                let bytes = parse_hex(&m.bytes)?;
                let mask = match &m.mask {
                    Some(s) => parse_hex(s)?,
                    None => vec![0xFF; bytes.len()],
                };
                if mask.len() != bytes.len() {
                    bail!("format '{}': mask length differs from bytes length", f.name);
                }
                sigs.push(Signature {
                    name: f.name.clone(),
                    offset: m.offset,
                    bytes,
                    mask,
                    priority: m.priority.unwrap_or(f.priority),
                });
            }
        }
        Ok(Detector { sigs })
    }

    pub fn detect_file(&self, path: &Path) -> io::Result<String> {
        let mut f = File::open(path)?;
        self.detect_reader(&mut f)
    }

    pub fn detect_reader<R: Read + Seek>(&self, r: &mut R) -> io::Result<String> {
        let len = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(0))?;
        let mut head = Vec::new();
        r.by_ref().take(HEAD as u64).read_to_end(&mut head)?;
        if head.is_empty() {
            return Ok("empty".into());
        }
        if starts_with_utf_bom(&head) {
            return Ok("text".into()); // UTF BOMs (e.g. FF FE) also look like an MPEG sync: decide here
        }

        let mut best: Option<&Signature> = None;
        for s in &self.sigs {
            if s.matches(&head) {
                best = match best {
                    None => Some(s),
                    Some(b) => {
                        if (s.priority, s.bytes.len()) > (b.priority, b.bytes.len()) {
                            Some(s)
                        } else {
                            Some(b)
                        }
                    }
                };
            }
        }

        let family = match best {
            Some(s) => s.name.as_str(),
            None => {
                let kind = if looks_like_text(&head) { "text" } else { "unknown" };
                return Ok(kind.to_string());
            }
        };
        refine(family, &head, len, r)
    }
}

fn refine<R: Read + Seek>(family: &str, head: &[u8], len: u64, r: &mut R) -> io::Result<String> {
    let out: String = match family {
        "zip" => refine_zip(r, len)?,
        "riff" => {
            let form = head.get(8..12).unwrap_or(&[]);
            let kind = if form == b"WAVE" {
                "wav"
            } else if form == b"AVI " {
                "avi"
            } else if form == b"WEBP" {
                "webp"
            } else {
                "riff"
            };
            kind.to_string()
        }
        "ebml" => {
            let window = &head[..head.len().min(64)];
            let kind = if window.windows(4).any(|w| w == b"webm") { "webm" } else { "mkv" };
            kind.to_string()
        }
        "ftyp" => ftyp_kind(head).to_string(),
        "cafebabe" => {
            // Mach-O fat: number of architectures (small); Java class: minor/major version (>= 45)
            let n = if head.len() >= 8 { u32::from_be_bytes([head[4], head[5], head[6], head[7]]) } else { u32::MAX };
            let kind = if n <= 30 { "macho_fat" } else { "class" };
            kind.to_string()
        }
        "mp3" => {
            let kind = if is_mp3(head) { "mp3" } else { "unknown" };
            kind.to_string()
        }
        "exe" => pe_kind(head, r)?.to_string(),
        "bmp" => {
            let ok = head.len() >= 18 && matches!(u32le(&head[14..18]), 12 | 40 | 52 | 56 | 64 | 108 | 124);
            let kind = if ok { "bmp" } else { "unknown" };
            kind.to_string()
        }
        "ico" => {
            let ok = head.len() >= 6 && (1..=256).contains(&u16le(&head[4..6]));
            let kind = if ok { "ico" } else { "unknown" };
            kind.to_string()
        }
        other => other.to_string(),
    };
    Ok(out)
}

fn ftyp_kind(head: &[u8]) -> &'static str {
    let b = head.get(8..12).unwrap_or(&[]);
    let is = |s: &[u8]| b == s;
    if is(b"avif") || is(b"avis") {
        "avif"
    } else if is(b"heic") || is(b"heix") || is(b"hevc") || is(b"heim") || is(b"heis") || is(b"mif1") || is(b"msf1") {
        "heic"
    } else if is(b"M4A ") || is(b"M4B ") || is(b"M4P ") {
        "m4a"
    } else if is(b"qt  ") {
        "mov"
    } else if b.starts_with(b"3g") {
        "3gp"
    } else {
        "mp4"
    }
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// UTF-8 / UTF-16 / UTF-32 byte-order marks: anything starting with a BOM is text,
/// regardless of what the signatures below would (mis)match ("FF FE" also satisfies
/// the MPEG sync pattern FF E0).
fn starts_with_utf_bom(head: &[u8]) -> bool {
    head.starts_with(&[0xEF, 0xBB, 0xBF]) // UTF-8
        || head.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) // UTF-32 BE
        || head.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) // UTF-32 LE
        || head.starts_with(&[0xFF, 0xFE]) // UTF-16 LE
        || head.starts_with(&[0xFE, 0xFF]) // UTF-16 BE
}

fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) || head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    let sample = &head[..head.len().min(8192)];
    if sample.contains(&0) {
        return false;
    }
    let controls = sample
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B))
        .count();
    controls * 100 < sample.len()
}

fn mp3_frame_ok(b: &[u8]) -> bool {
    if b.len() < 4 {
        return false;
    }
    b[0] == 0xFF
        && (b[1] & 0xE0) == 0xE0
        && ((b[1] >> 3) & 3) != 1 // reserved MPEG version
        && ((b[1] >> 1) & 3) != 0 // reserved layer
        && (b[2] >> 4) != 0xF // bad bitrate
        && ((b[2] >> 2) & 3) != 3 // reserved sample rate
}

fn is_mp3(head: &[u8]) -> bool {
    let mut pos = 0usize;
    if head.starts_with(b"ID3") {
        if head.len() < 10 {
            return false;
        }
        let size = ((head[6] as usize & 0x7F) << 21)
            | ((head[7] as usize & 0x7F) << 14)
            | ((head[8] as usize & 0x7F) << 7)
            | (head[9] as usize & 0x7F);
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        pos = 10 + size + footer;
        while pos < head.len() && head[pos] == 0 {
            pos += 1;
        }
        if pos >= head.len() {
            return true; // tag (cover art etc.) larger than our window: trust the ID3 header
        }
    }
    let len = mp3_frame_len(&head[pos..]);
    if len == 0 {
        return false;
    }
    // Require the next frame header to line up exactly at the computed frame length.
    // A real MP3 is a run of consecutive frames; UTF-16 text or stray FF/FF byte pairs
    // never repeat there, so this kills the false positives (e.g. FF FE 44 00 / FF FF 86 00).
    let next = pos + len;
    if next + 4 > head.len() {
        return false; // too short to hold a second frame header: not convincing
    }
    mp3_frame_ok(&head[next..])
}

/// Byte length of the MPEG audio frame starting at `b`, 0 if the header is invalid.
/// Bitrate/samplerate tables are MPEG1; for MPEG2 / 2.5 the values are halved (an
/// approximation is plenty for the frame-layout heuristic).
fn mp3_frame_len(b: &[u8]) -> usize {
    if b.len() < 4 || !mp3_frame_ok(b) {
        return 0;
    }
    let ver = (b[1] >> 3) & 3; // 0 = 2.5, 2 = MPEG2, 3 = MPEG1
    let layer = (b[1] >> 1) & 3; // 1 = III, 2 = II, 3 = I
    let br = (b[2] >> 4) as usize;
    let sr = ((b[2] >> 2) & 3) as usize;
    let pad = (b[2] & 1) as usize;
    let l1 = [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448];
    let l2 = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
    let l3 = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    let table = match layer {
        3 => l1,
        2 => l2,
        _ => l3,
    };
    let rate = table[br];
    let sample = [44100, 48000, 32000][sr];
    let rate = if ver == 3 { rate } else { rate / 2 }; // kbps; MPEG2/2.5 halves it
    if rate == 0 {
        return 0;
    }
    if layer == 3 {
        (12 * rate * 1000 / sample + pad) * 4 // Layer I: 4 bytes per slot
    } else {
        144 * rate * 1000 / sample + pad // Layer II / III
    }
}

/// "exe" | "dll" for a valid PE image, "mz" for other MZ files (DOS/NE).
fn pe_kind<R: Read + Seek>(head: &[u8], r: &mut R) -> io::Result<&'static str> {
    if head.len() < 0x40 {
        return Ok("mz");
    }
    let off = u32le(&head[0x3C..0x40]) as u64;
    r.seek(SeekFrom::Start(off))?;
    let mut buf = [0u8; 24];
    match r.read_exact(&mut buf) {
        Ok(()) => {
            if &buf[..4] != b"PE\0\0" {
                return Ok("mz");
            }
            let characteristics = u16le(&buf[22..24]);
            Ok(if characteristics & 0x2000 != 0 { "dll" } else { "exe" })
        }
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok("mz"),
        Err(e) => Err(e),
    }
}

fn refine_zip<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<String> {
    let names = match zip_central_names(r, len)? {
        Some(n) => n,
        None => return Ok("zip".into()),
    };
    let has = |s: &str| names.iter().any(|n| n == s);
    let prefix = |p: &str| names.iter().any(|n| n.starts_with(p));
    let kind = if has("[Content_Types].xml") {
        if prefix("xl/") {
            "xlsx"
        } else if prefix("ppt/") {
            "pptx"
        } else {
            "docx" // word/ or any other OOXML package
        }
    } else if has("AndroidManifest.xml") {
        "apk"
    } else if has("mimetype") && has("META-INF/manifest.xml") {
        "odf"
    } else if has("mimetype") && has("META-INF/container.xml") {
        "epub"
    } else if has("META-INF/MANIFEST.MF") {
        "jar"
    } else {
        "zip"
    };
    Ok(kind.to_string())
}

/// Entry names from the ZIP central directory (None if it cannot be parsed, e.g. ZIP64 or truncated).
fn zip_central_names<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<Option<Vec<String>>> {
    let tail_len = len.min(65_557);
    r.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    r.read_exact(&mut tail)?;
    let eocd = match tail.windows(4).rposition(|w| w == b"PK\x05\x06") {
        Some(p) => p,
        None => return Ok(None),
    };
    if tail.len() < eocd + 22 {
        return Ok(None);
    }
    let e = &tail[eocd..];
    let cd_size = u32le(&e[12..16]) as u64;
    let cd_off = u32le(&e[16..20]) as u64;
    if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF || cd_size > 4 * 1024 * 1024 || cd_off + cd_size > len {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(cd_off))?;
    let mut cd = vec![0u8; cd_size as usize];
    r.read_exact(&mut cd)?;

    let mut names = Vec::new();
    let mut p = 0usize;
    while p + 46 <= cd.len() && &cd[p..p + 4] == b"PK\x01\x02" {
        let n = u16le(&cd[p + 28..p + 30]) as usize;
        let x = u16le(&cd[p + 30..p + 32]) as usize;
        let c = u16le(&cd[p + 32..p + 34]) as usize;
        if p + 46 + n > cd.len() {
            break;
        }
        names.push(String::from_utf8_lossy(&cd[p + 46..p + 46 + n]).into_owned());
        p += 46 + n + x + c;
    }
    Ok(Some(names))
}


/// Detects the format of an in-memory file by its magic bytes (never by extension).
pub fn detect(data: &[u8]) -> String {
    static DETECTOR: OnceLock<Detector> = OnceLock::new();
    let d = DETECTOR.get_or_init(|| Detector::new().expect("embedded format table is valid"));
    d.detect_reader(&mut io::Cursor::new(data)).unwrap_or_else(|_| "unknown".into())
}
