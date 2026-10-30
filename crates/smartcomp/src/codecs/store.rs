use anyhow::{bail, Result};

use super::{Codec, Level};

pub struct Store;

impl Codec for Store {
    fn id(&self) -> &'static str {
        "store"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn compress(&self, data: &[u8], _level: Level) -> Result<Vec<u8>> {
        Ok(data.to_vec())
    }

    fn decompress(&self, data: &[u8], original_len: usize) -> Result<Vec<u8>> {
        if data.len() != original_len {
            bail!("stored payload has {} bytes, expected {original_len}", data.len());
        }
        Ok(data.to_vec())
    }
}
