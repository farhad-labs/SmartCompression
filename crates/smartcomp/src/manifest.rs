use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub far_version: String,
    pub level: u8,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: u32,
    pub path: String,
    pub format: String,
    pub tool: String,
    pub tool_version: String,
    pub tool_hash: String,
    pub level: u8,
    pub original_size: u64,
    /// Size of the codec output, before encryption.
    pub compressed_size: u64,
    pub blake3: String,
    /// "tool", "store" or "fallback".
    pub method: String,
    pub stage: String,
    pub payload: String,
    pub fallback_reason: Option<String>,
    /// Where the payload blob lives in the archive file (after encryption, if any).
    pub offset: u64,
    pub stored_size: u64,
    #[serde(default)]
    pub params: String,
}

pub fn payload_name(id: u32) -> String {
    format!("payload/{id:08}")
}
