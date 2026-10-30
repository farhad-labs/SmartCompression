//! On-disk layout:
//!   header  : "SMCA" | u16 version | u8 flags | [u8;32] salt
//!   blobs   : payloads, then the manifest blob (id 0)
//!   footer  : u64 manifest_off | u64 manifest_len | [u8;32] blake3(stored manifest blob) | "SMCE"

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use rand::{rngs::OsRng, RngCore};

use crate::crypto::Cipher;

const MAGIC: &[u8; 4] = b"SMCA";
const END_MAGIC: &[u8; 4] = b"SMCE";
const VERSION: u16 = 1;
const FLAG_ENCRYPTED: u8 = 1;
const HEADER_LEN: u64 = 4 + 2 + 1 + 32;
const FOOTER_LEN: u64 = 8 + 8 + 32 + 4;
pub const MANIFEST_ID: u32 = 0;

pub struct Writer<W: Write> {
    w: W,
    pos: u64,
    cipher: Option<Cipher>,
}

impl<W: Write> Writer<W> {
    pub fn new(mut w: W, password: Option<&str>) -> Result<Self> {
        let mut salt = [0u8; 32];
        let mut flags = 0;
        let cipher = match password {
            Some(pw) => {
                OsRng.fill_bytes(&mut salt);
                flags |= FLAG_ENCRYPTED;
                Some(Cipher::derive(pw, &salt)?)
            }
            None => None,
        };
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&[flags])?;
        w.write_all(&salt)?;
        Ok(Writer { w, pos: HEADER_LEN, cipher })
    }

    /// Writes one blob and returns (offset, stored length, hash of the stored bytes).
    fn put(&mut self, id: u32, data: &[u8]) -> Result<(u64, u64, [u8; 32])> {
        let off = self.pos;
        let stored: std::borrow::Cow<[u8]> = match &self.cipher {
            Some(c) => c.encrypt(id, data)?.into(),
            None => data.into(),
        };
        self.w.write_all(&stored)?;
        self.pos += stored.len() as u64;
        Ok((off, stored.len() as u64, *blake3::hash(&stored).as_bytes()))
    }

    /// Returns (offset, stored length).
    pub fn add_blob(&mut self, id: u32, data: &[u8]) -> Result<(u64, u64)> {
        let (off, len, _) = self.put(id, data)?;
        Ok((off, len))
    }

    pub fn finish(mut self, manifest_json: &[u8]) -> Result<()> {
        // The footer hash covers the stored bytes, so it can be checked before decrypting.
        let (off, len, hash) = self.put(MANIFEST_ID, manifest_json)?;
        self.w.write_all(&off.to_le_bytes())?;
        self.w.write_all(&len.to_le_bytes())?;
        self.w.write_all(&hash)?;
        self.w.write_all(END_MAGIC)?;
        self.w.flush()?;
        Ok(())
    }
}

pub struct Reader {
    f: File,
    cipher: Option<Cipher>,
}

impl Reader {
    /// Opens the archive and returns the decoded manifest JSON.
    pub fn open(path: &Path, password: Option<&str>) -> Result<(Self, Vec<u8>)> {
        let mut f = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let size = f.metadata()?.len();
        if size < HEADER_LEN + FOOTER_LEN {
            bail!("not a smartcomp archive (too small)");
        }
        let mut head = [0u8; HEADER_LEN as usize];
        f.read_exact(&mut head)?;
        if &head[..4] != MAGIC {
            bail!("not a smartcomp archive (bad magic)");
        }
        let version = u16::from_le_bytes([head[4], head[5]]);
        if version != VERSION {
            bail!("unsupported archive version {version}");
        }
        let cipher = if head[6] & FLAG_ENCRYPTED != 0 {
            let pw = password.context("archive is encrypted: a password is required (-p)")?;
            let salt: [u8; 32] = head[7..39].try_into().unwrap();
            Some(Cipher::derive(pw, &salt)?)
        } else {
            None
        };

        f.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
        let mut foot = [0u8; FOOTER_LEN as usize];
        f.read_exact(&mut foot)?;
        if &foot[48..] != END_MAGIC {
            bail!("archive is truncated or corrupted (bad footer)");
        }
        let off = u64::from_le_bytes(foot[0..8].try_into().unwrap());
        let len = u64::from_le_bytes(foot[8..16].try_into().unwrap());
        if off < HEADER_LEN || off.checked_add(len).map_or(true, |e| e > size - FOOTER_LEN) {
            bail!("archive is corrupted (manifest out of range)");
        }

        let mut r = Reader { f, cipher };
        let stored = r.read_stored(off, len)?;
        if blake3::hash(&stored).as_bytes() != &foot[16..48] {
            bail!("archive is corrupted (manifest hash mismatch)");
        }
        let manifest = match &r.cipher {
            Some(c) => c.decrypt(MANIFEST_ID, &stored)?,
            None => stored,
        };
        Ok((r, manifest))
    }

    fn read_stored(&mut self, off: u64, len: u64) -> Result<Vec<u8>> {
        self.f.seek(SeekFrom::Start(off))?;
        let mut buf = vec![0u8; len as usize];
        self.f.read_exact(&mut buf).context("archive is truncated")?;
        Ok(buf)
    }

    pub fn read_blob(&mut self, id: u32, off: u64, len: u64) -> Result<Vec<u8>> {
        let stored = self.read_stored(off, len)?;
        match &self.cipher {
            Some(c) => c.decrypt(id, &stored),
            None => Ok(stored),
        }
    }
}
