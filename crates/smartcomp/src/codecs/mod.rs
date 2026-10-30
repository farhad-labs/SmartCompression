//! In-memory codecs. A codec compresses one block into a self-describing blob (any
//! parameters needed to decode are stored inside it); `pack`/`unpack` split larger
//! inputs into blocks, which also lets one big file use several cores.

use anyhow::{bail, ensure, Result};
use rayon::prelude::*;

mod brotli;
mod lepton;
mod lzma2;
mod nzcm;
mod packmp3;
mod ppmd7;
mod precomp;
mod store;
mod uharc;
mod zstd;

pub use brotli::Brotli;
pub use lepton::Lepton;
pub use lzma2::Lzma2;
pub use nzcm::NzCm;
pub use packmp3::PackMp3;
pub use ppmd7::Ppmd7;
pub use precomp::{child_main as precomp_child_main, Precomp};
pub use store::Store;
pub use uharc::Uharc;
pub use zstd::Zstd;

pub const BLOCK_SIZE: usize = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Store = 1,
    Medium = 2,
    /// Strongest of the fast-enough coders: sample-probed PPMd / UHARC / LZMA2 / BCJ.
    Ultra = 3,
    /// Maximum ratio: adds nanozip's context-mixing coder, which is several times slower.
    HighUltra = 4,
}

impl Level {
    /// True for the levels that use large dictionaries and the slowest settings of each coder.
    pub fn is_ultra(self) -> bool {
        matches!(self, Level::Ultra | Level::HighUltra)
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Store => "store",
            Level::Medium => "medium",
            Level::Ultra => "ultra",
            Level::HighUltra => "highultra",
        }
    }
}

pub trait Codec: Sync {
    fn id(&self) -> &'static str;
    fn version(&self) -> &'static str;
    /// Name used to request this exact configuration (`by_spec`); defaults to the id.
    fn spec(&self) -> &'static str {
        self.id()
    }
    /// Human-readable settings, recorded in the manifest.
    fn params(&self, _level: Level) -> String {
        String::new()
    }
    /// False for transforms that need the whole file (JPEG, MP3): never cut into blocks.
    fn splittable(&self) -> bool {
        true
    }
    /// Compresses one block (never empty) into a self-describing blob.
    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>>;
    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>>;
}

static STORE: Store = Store;
static LZMA2: Lzma2 = Lzma2 { bcj: false, lc: -1, lp: -1, pb: -1, spec: "lzma2" };
static LZMA2_PCM: Lzma2 = Lzma2 { bcj: false, lc: 0, lp: 1, pb: 1, spec: "lzma2:pcm" };
static LZMA2_REC: Lzma2 = Lzma2 { bcj: false, lc: 0, lp: 2, pb: 2, spec: "lzma2:rec" };
static LZMA2_BCJ: Lzma2 = Lzma2 { bcj: true, lc: -1, lp: -1, pb: -1, spec: "lzma2-bcj" };
static PPMD7_O8: Ppmd7 = Ppmd7 { order: 8, spec: "ppmd7:8" };
static PPMD7_O16: Ppmd7 = Ppmd7 { order: 16, spec: "ppmd7:16" };
static PPMD7_O32: Ppmd7 = Ppmd7 { order: 32, spec: "ppmd7:32" };
static ZSTD: Zstd = Zstd;
static BROTLI: Brotli = Brotli;
static LEPTON: Lepton = Lepton;
static NZCM: NzCm = NzCm;
static PRECOMP: Precomp = Precomp;
static PACKMP3: PackMp3 = PackMp3;
static UHARC_MX: Uharc = Uharc { mode: sc_uharc::Mode::Ppm };
static UHARC_M3: Uharc = Uharc { mode: sc_uharc::Mode::Alz3 };

pub fn by_id(id: &str) -> Result<&'static dyn Codec> {
    Ok(match id {
        "store" => &STORE,
        "lzma2" => &LZMA2,
        "lzma2-bcj" => &LZMA2_BCJ,
        "ppmd7" => &PPMD7_O8, // decoding reads the order from the blob
        "zstd" => &ZSTD,
        "brotli" => &BROTLI,
        "lepton" => &LEPTON,
        "nzcm" => &NZCM,
        "precomp" => &PRECOMP,
        "packmp3" => &PACKMP3,
        "uharc-mx" => &UHARC_MX,
        "uharc-m3" => &UHARC_M3,
        other => bail!("archive uses codec '{other}', which this build does not support"),
    })
}

/// Looks up one exact configuration, e.g. `ppmd7:16` or `lzma2:pcm`.
pub fn by_spec(spec: &str) -> Result<&'static dyn Codec> {
    Ok(match spec {
        "lzma2:pcm" => &LZMA2_PCM,
        "lzma2:rec" => &LZMA2_REC,
        "ppmd7:8" => &PPMD7_O8,
        "ppmd7:16" => &PPMD7_O16,
        "ppmd7:32" => &PPMD7_O32,
        other => by_id(other)?,
    })
}

/// Deterministic identity of a codec configuration, recorded as `tool_hash`.
pub fn tool_hash(codec: &dyn Codec, params: &str) -> String {
    blake3::hash(format!("{}@{}:{}", codec.id(), codec.version(), params).as_bytes()).to_hex().to_string()
}

/// Splits `data` into blocks and frames them as `[u32 raw_len][u32 blob_len][blob]...`.
pub fn pack(codec: &dyn Codec, data: &[u8], level: Level) -> Result<Vec<u8>> {
    let block = if codec.splittable() { BLOCK_SIZE } else { usize::MAX };
    let blobs = data
        .par_chunks(block)
        .map(|b| codec.compress(b, level))
        .collect::<Result<Vec<_>>>()?;
    ensure!(data.len() <= u32::MAX as usize, "file too large for the block framing");
    let mut out = Vec::with_capacity(blobs.iter().map(|b| b.len() + 8).sum());
    for (raw, blob) in data.chunks(block).zip(&blobs) {
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(blob);
    }
    Ok(out)
}

pub fn unpack(codec: &dyn Codec, packed: &[u8], original_len: usize) -> Result<Vec<u8>> {
    let mut frames = Vec::new();
    let mut pos = 0usize;
    let mut total = 0usize;
    while pos < packed.len() {
        ensure!(packed.len() - pos >= 8, "truncated block header");
        let raw = u32::from_le_bytes(packed[pos..pos + 4].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(packed[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        ensure!(len <= packed.len() - pos, "truncated block");
        total = total.checked_add(raw).filter(|t| *t <= original_len).ok_or_else(|| anyhow::anyhow!("block sizes exceed the file size"))?;
        frames.push((raw, &packed[pos..pos + len]));
        pos += len;
    }
    ensure!(total == original_len, "blocks add up to {total} bytes, expected {original_len}");
    let parts = frames
        .par_iter()
        .map(|(raw, blob)| codec.decompress(blob, *raw))
        .collect::<Result<Vec<_>>>()?;
    Ok(parts.concat())
}
