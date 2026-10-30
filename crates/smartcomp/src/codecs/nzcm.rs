use std::sync::{Condvar, Mutex};

use anyhow::{bail, ensure, Result};

use super::{Codec, Level};

/// Context-mixing coder from nanozip-re (`-cc`'s core model). Strong on binary data, databases
/// and raw audio, and slow. Blob layout: `[a_bits][b_bits][arithmetic-coded payload]`.
pub struct NzCm;

/// Table sizes for a block: bigger tables only pay off on bigger inputs, and they cost memory.
fn bits_for(n: usize) -> (i32, i32) {
    match n {
        0..=0x10_0000 => (24, 22),
        0x10_0001..=0x80_0000 => (26, 23),
        _ => (28, 25),
    }
}

/// Approximate resident bytes of one model: hash table + context table + window + fixed tables.
fn footprint(a: i32, b: i32, n: usize) -> usize {
    (1usize << a) + 4 * (1usize << b) + n + (32 << 20)
}

/// Models are big, so cap the memory they may use at once (all callers share this budget).
struct Budget {
    used: Mutex<usize>,
    freed: Condvar,
}

static BUDGET: Budget = Budget { used: Mutex::new(0), freed: Condvar::new() };
const BUDGET_BYTES: usize = 2304 << 20;

struct Lease(usize);

fn lease(bytes: usize) -> Lease {
    let want = bytes.min(BUDGET_BYTES); // one oversized model must still be able to run, alone
    let mut used = BUDGET.used.lock().unwrap();
    while *used + want > BUDGET_BYTES {
        used = BUDGET.freed.wait(used).unwrap();
    }
    *used += want;
    Lease(want)
}

impl Drop for Lease {
    fn drop(&mut self) {
        *BUDGET.used.lock().unwrap() -= self.0;
        BUDGET.freed.notify_all();
    }
}

fn status(rc: i32) -> Result<()> {
    match rc {
        0 => Ok(()),
        1 => bail!("nanozip CM: output buffer too small"),
        2 => bail!("nanozip CM: out of memory"),
        _ => bail!("nanozip CM: internal error"),
    }
}

impl Codec for NzCm {
    fn id(&self) -> &'static str {
        "nzcm"
    }

    fn version(&self) -> &'static str {
        "nanozip-re 0.09a"
    }

    fn params(&self, _level: Level) -> String {
        "cm a=28 b=25 (scaled down for small blocks)".into()
    }

    fn compress(&self, block: &[u8], _level: Level) -> Result<Vec<u8>> {
        ensure!(block.len() < u32::MAX as usize / 2, "block too large");
        let (a, b) = bits_for(block.len());
        let _lease = lease(footprint(a, b, block.len()));
        let cap = block.len() + block.len() / 8 + 1024;
        let mut out = vec![0u8; 2 + cap];
        let mut len = 0u32;
        let rc = unsafe { sc_nanozip_sys::sc_nzcm_encode(block.as_ptr(), block.len() as u32, a, b, out[2..].as_mut_ptr(), cap as u32, &mut len) };
        status(rc)?;
        out[0] = a as u8;
        out[1] = b as u8;
        out.truncate(2 + len as usize);
        Ok(out)
    }

    fn decompress(&self, blob: &[u8], original_len: usize) -> Result<Vec<u8>> {
        ensure!(blob.len() >= 2, "nanozip CM blob too short");
        let (a, b) = (blob[0] as i32, blob[1] as i32);
        ensure!((12..=30).contains(&a) && (10..=28).contains(&b), "invalid nanozip CM parameters");
        ensure!(original_len < u32::MAX as usize / 2 && blob.len() < u32::MAX as usize, "block too large");
        let _lease = lease(footprint(a, b, original_len));
        let mut out = vec![0u8; original_len];
        let rc = unsafe { sc_nanozip_sys::sc_nzcm_decode(blob[2..].as_ptr(), (blob.len() - 2) as u32, a, b, out.as_mut_ptr(), original_len as u32) };
        status(rc)?;
        Ok(out)
    }
}
