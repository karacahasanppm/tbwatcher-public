//! Read-only local save reader (DESIGN.md §10a) — decrypts the player's EasySave3 save and aggregates
//! the in-game stash into per-item counts. **Never writes the save.** [decision-save-read-only]

mod mapping;
pub use mapping::{category, grade, icon, market_hashes};

use std::collections::HashMap;
use std::path::PathBuf;

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use serde::{Deserialize, Serialize};

type Aes128CbcDec = cbc::Decryptor<Aes128>;

/// The game's EasySave3 password — a public constant (the same one the MIT `tbh-copilot` uses).
const ES3_PASSWORD: &[u8] = b"emuMqG3bLYJ938ZDCfieWJ";

#[derive(Debug)]
pub enum SaveError {
    NotFound,
    Io(String),
    Decrypt(String),
    Parse(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NotFound => write!(f, "save file not found"),
            SaveError::Io(e) => write!(f, "cannot read save: {e}"),
            SaveError::Decrypt(e) => write!(f, "cannot decrypt save (format changed?): {e}"),
            SaveError::Parse(e) => write!(f, "cannot parse save (format changed?): {e}"),
        }
    }
}

/// One stacked stash line: an item type and how many the player holds across all stores.
#[derive(Debug, Clone, Serialize)]
pub struct StashCount {
    pub item_key: i64,
    pub count: u64,
}

/// Reads the local save read-only. The path is resolved once; the file is never modified.
pub struct SaveReader {
    path: PathBuf,
}

impl SaveReader {
    /// Default: `%USERPROFILE%\AppData\LocalLow\TesseractStudio\TaskbarHero\SaveFile_Live.es3`.
    pub fn new() -> Self {
        let base = std::env::var("USERPROFILE").unwrap_or_default();
        let path = PathBuf::from(base)
            .join("AppData")
            .join("LocalLow")
            .join("TesseractStudio")
            .join("TaskbarHero")
            .join("SaveFile_Live.es3");
        Self { path }
    }

    #[cfg(test)]
    fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn read_stash(&self) -> Result<Vec<StashCount>, SaveError> {
        match read_path(&self.path) {
            // The game writes the live file concurrently; a read caught mid-rotation can come back torn
            // (Io/Decrypt/Parse). Fall back once to the sibling `.bak` snapshot — still read-only.
            Err(e) if !matches!(e, SaveError::NotFound) => read_path(&self.backup_path()).or(Err(e)),
            other => other,
        }
    }

    fn backup_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push(".bak");
        PathBuf::from(p)
    }
}

fn read_path(path: &std::path::Path) -> Result<Vec<StashCount>, SaveError> {
    if !path.exists() {
        return Err(SaveError::NotFound);
    }
    let bytes = std::fs::read(path).map_err(|e| SaveError::Io(e.to_string()))?;
    let json = decrypt_es3(&bytes)?;
    parse_stash(&json)
}

/// ES3 AES-128-CBC: IV is the first 16 bytes; key = PBKDF2-HMAC-SHA1(password, salt = IV, 100 iters);
/// PKCS7 padding. (Verified against a real save.)
fn decrypt_es3(bytes: &[u8]) -> Result<Vec<u8>, SaveError> {
    if bytes.len() <= 16 {
        return Err(SaveError::Decrypt("file too short".into()));
    }
    let (iv, ct) = bytes.split_at(16);
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(ES3_PASSWORD, iv, 100, &mut key);
    Aes128CbcDec::new_from_slices(&key, iv)
        .map_err(|_| SaveError::Decrypt("bad key/iv length".into()))?
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|_| SaveError::Decrypt("padding/length error".into()))
}

fn parse_stash(json: &[u8]) -> Result<Vec<StashCount>, SaveError> {
    let root: Es3Root = serde_json::from_slice(json).map_err(|e| SaveError::Parse(e.to_string()))?;
    let player: PlayerSave =
        serde_json::from_str(&root.player.value).map_err(|e| SaveError::Parse(e.to_string()))?;
    Ok(aggregate(&player))
}

#[derive(Deserialize)]
struct Es3Root {
    #[serde(rename = "PlayerSaveData")]
    player: Es3Value,
}
#[derive(Deserialize)]
struct Es3Value {
    value: String,
}
#[derive(Deserialize)]
struct PlayerSave {
    #[serde(rename = "itemSaveDatas")]
    items: Vec<ItemInstance>,
    #[serde(rename = "inventorySaveDatas", default)]
    inventory: Vec<Slot>,
    #[serde(rename = "stashSaveDatas", default)]
    stash: Vec<Slot>,
    #[serde(rename = "remakeTradingStashSaveDatas", default)]
    trading: Vec<Slot>,
}
#[derive(Deserialize)]
struct ItemInstance {
    #[serde(rename = "ItemKey")]
    item_key: i64,
    #[serde(rename = "UniqueId")]
    unique_id: i64,
}
#[derive(Deserialize)]
struct Slot {
    #[serde(rename = "ItemUniqueId")]
    item_unique_id: i64,
}

/// Resolve each filled slot (inventory + stash + trading) to its item type, and count per type.
fn aggregate(player: &PlayerSave) -> Vec<StashCount> {
    let by_uid: HashMap<i64, i64> = player
        .items
        .iter()
        .map(|i| (i.unique_id, i.item_key))
        .collect();

    let mut counts: HashMap<i64, u64> = HashMap::new();
    for slot in player
        .inventory
        .iter()
        .chain(&player.stash)
        .chain(&player.trading)
    {
        if let Some(&item_key) = by_uid.get(&slot.item_unique_id) {
            *counts.entry(item_key).or_insert(0) += 1;
        }
    }

    let mut out: Vec<StashCount> = counts
        .into_iter()
        .map(|(item_key, count)| StashCount { item_key, count })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.item_key.cmp(&b.item_key)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_and_aggregates_a_recorded_es3_save() {
        let bytes = include_bytes!("../../fixtures/es3-sample.bin");
        let json = decrypt_es3(bytes).expect("decrypts");
        let stash = parse_stash(&json).expect("parses");
        // 110001 appears twice (uids 1,2); 142002 once (uid 3); uid 99 has no instance → skipped.
        assert_eq!(stash.len(), 2);
        assert_eq!(stash[0].item_key, 110001);
        assert_eq!(stash[0].count, 2);
        assert!(stash.iter().any(|s| s.item_key == 142002 && s.count == 1));
    }

    #[test]
    fn falls_back_to_bak_when_live_is_torn() {
        let good = include_bytes!("../../fixtures/es3-sample.bin");
        let dir = std::env::temp_dir().join(format!("tbw-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("SaveFile_Live.es3");
        let bak = dir.join("SaveFile_Live.es3.bak");
        std::fs::write(&live, b"torn half-written garbage").unwrap();
        std::fs::write(&bak, good).unwrap();

        let stash = SaveReader::with_path(live).read_stash().expect("falls back to .bak");
        assert_eq!(stash[0].item_key, 110001);

        std::fs::remove_dir_all(&dir).ok();
    }
}
