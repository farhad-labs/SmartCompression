//! Precomp 0.4.8 as a transform: deflate/zlib streams inside PNG, GIF, ZIP-family and gzip files
//! are re-inflated (and can be re-deflated bit-exactly), and the result is compressed by a
//! normal coder. Precomp keeps global state, writes temp files into the working directory and
//! calls `exit()` on errors, so it only ever runs in a child process of this program (the hidden
//! `__precomp` mode) with its own temporary directory.
//!
//! Blob layout: `[stage byte][u32 pcf length][stage blob]`, stage 0 = LZMA2, 1 = nanozip CM.

use std::ffi::CString;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};

use super::{by_id, Codec, Level};

pub struct Precomp;

const STAGE_LZMA2: u8 = 0;
const STAGE_NZCM: u8 = 1;
/// Nanozip's CM is the strongest stage but slow; keep it for outputs that can afford it.
const NZCM_MAX_PCF: usize = 48 << 20;

fn stage_codec(stage: u8) -> Result<&'static dyn Codec> {
    match stage {
        STAGE_LZMA2 => by_id("lzma2"),
        STAGE_NZCM => by_id("nzcm"),
        _ => bail!("unknown precomp stage {stage}"),
    }
}

/// Entry point of the hidden `__precomp <0|1> <in> <out>` mode. Returns the process exit code.
pub fn child_main(args: &[String]) -> i32 {
    let [mode, input, output] = args else { return 2 };
    let (Ok(input), Ok(output)) = (CString::new(input.as_str()), CString::new(output.as_str())) else { return 2 };
    let mut msg = [0 as std::os::raw::c_char; 512];
    let mode = if mode == "0" { 0 } else { 1 };
    let rc = unsafe { sc_precomp_sys::sc_precomp_run(mode, input.as_ptr(), output.as_ptr(), msg.as_mut_ptr()) };
    if rc != 0 {
        let text = unsafe { std::ffi::CStr::from_ptr(msg.as_ptr()) }.to_string_lossy().into_owned();
        eprintln!("precomp: {text}");
    }
    rc
}

/// The executable that understands `__precomp`: normally this program itself.
fn worker_exe() -> Result<std::path::PathBuf> {
    match std::env::var_os("SMARTCOMP_EXE") {
        Some(p) => Ok(p.into()),
        None => std::env::current_exe().context("cannot locate the smartcomp executable"),
    }
}

fn run_child(mode: &str, dir: &Path, input: &str, output: &str) -> Result<()> {
    let out = Command::new(worker_exe()?)
        .arg("__precomp")
        .args([mode, input, output])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .context("cannot start the precomp worker")?;
    ensure!(out.status.success(), "precomp worker failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

/// Runs precomp in a fresh directory: `data` in, output file bytes out.
fn transform(mode: &str, data: &[u8]) -> Result<Vec<u8>> {
    let dir = tempfile::tempdir().context("cannot create a temporary directory")?;
    std::fs::write(dir.path().join("in.dat"), data)?;
    run_child(mode, dir.path(), "in.dat", "out.dat")?;
    std::fs::read(dir.path().join("out.dat")).context("precomp produced no output")
}

impl Codec for Precomp {
    fn id(&self) -> &'static str {
        "precomp"
    }

    fn version(&self) -> &'static str {
        "0.4.8"
    }

    fn splittable(&self) -> bool {
        false
    }

    fn params(&self, level: Level) -> String {
        format!("stage={}", if level == Level::HighUltra { "nzcm|lzma2" } else { "lzma2" })
    }

    fn compress(&self, block: &[u8], level: Level) -> Result<Vec<u8>> {
        let pcf = transform("0", block)?;
        // Precomp can report success yet fail to reproduce the file, so prove the round trip.
        ensure!(transform("1", &pcf)? == block, "precomp could not restore the file exactly");
        ensure!(pcf.len() < u32::MAX as usize, "precompressed data too large");
        let stage = if level == Level::HighUltra && pcf.len() <= NZCM_MAX_PCF { STAGE_NZCM } else { STAGE_LZMA2 };
        let packed = stage_codec(stage)?.compress(&pcf, level)?;
        let mut out = Vec::with_capacity(5 + packed.len());
        out.push(stage);
        out.extend_from_slice(&(pcf.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        ensure!(blob.len() >= 5, "precomp blob too short");
        let pcf_len = u32::from_le_bytes(blob[1..5].try_into().unwrap()) as usize;
        let pcf = stage_codec(blob[0])?.decompress(&blob[5..], pcf_len)?;
        let data = transform("1", &pcf)?;
        ensure!(data.len() == original_len, "precomp restored {} bytes, expected {original_len}", data.len());
        Ok(data)
    }
}
