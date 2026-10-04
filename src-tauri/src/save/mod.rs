//! Read-only local save reader (DESIGN.md §10a) — decrypts the player's EasySave3 save and aggregates
//! the in-game stash into per-item counts. **Never writes the save.** [decision-save-read-only]

mod mapping;
pub use mapping::{category, describe, grade, hero_name, icon, market_hashes, set_map_path, Described};

use std::collections::HashMap;
use std::path::PathBuf;

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use serde::{Deserialize, Deserializer, Serialize};

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

/// One equipped item and the materials applied to it (gems, engraving parts, inscription scrolls),
/// grouped by material with a count.
#[derive(Debug, Clone, Serialize)]
pub struct EquippedItem {
    pub item_key: i64,
    pub materials: Vec<StashCount>,
}

/// An unlocked hero's equipped gear (heroes with nothing equipped are omitted).
#[derive(Debug, Clone, Serialize)]
pub struct HeroGear {
    pub hero_key: i64,
    pub level: i64,
    pub items: Vec<EquippedItem>,
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
        self.load().map(|p| aggregate(&p))
    }

    /// Each unlocked hero's equipped gear and what's applied to it. Equipped items live outside the
    /// inventory/stash slots (on the hero), so the stash never counts them.
    pub fn read_equipment(&self) -> Result<Vec<HeroGear>, SaveError> {
        self.load().map(|p| equipment(&p))
    }

    fn load(&self) -> Result<PlayerSave, SaveError> {
        // The game rewrites the live file while running, so a read can land mid-rotation: torn content, or
        // the live file briefly absent. Fall back once to the sibling `.bak` snapshot (still read-only),
        // keeping the original error — so it's NotFound only when neither file exists.
        read_path(&self.path).or_else(|e| read_path(&self.backup_path()).map_err(|_| e))
    }

    fn backup_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push(".bak");
        PathBuf::from(p)
    }
}

fn read_path(path: &std::path::Path) -> Result<PlayerSave, SaveError> {
    if !path.exists() {
        return Err(SaveError::NotFound);
    }
    let bytes = std::fs::read(path).map_err(|e| SaveError::Io(e.to_string()))?;
    let json = decrypt_es3(&bytes)?;
    parse_player(&json)
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

fn parse_player(json: &[u8]) -> Result<PlayerSave, SaveError> {
    let root: Es3Root = serde_json::from_slice(json).map_err(|e| SaveError::Parse(e.to_string()))?;
    serde_json::from_str(&root.player.value).map_err(|e| SaveError::Parse(e.to_string()))
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
    #[serde(rename = "heroSaveDatas", default, deserialize_with = "lenient")]
    heroes: Vec<Hero>,
}

/// Parse a field only the gear screen needs without letting a surprise shape (null, a renamed sub-field)
/// fail the whole save and take the stash down with it: anything unexpected reads as empty.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).unwrap_or_default())
}
#[derive(Deserialize)]
struct ItemInstance {
    #[serde(rename = "ItemKey")]
    item_key: i64,
    #[serde(rename = "UniqueId")]
    unique_id: i64,
    /// The materials currently applied to the item; an empty/unused entry has `MaterialKey` 0.
    #[serde(rename = "EnchantData", default, deserialize_with = "lenient")]
    enchants: Vec<Enchant>,
}
#[derive(Deserialize)]
struct Enchant {
    #[serde(rename = "MaterialKey", default)]
    material_key: i64,
}
#[derive(Deserialize)]
struct Hero {
    #[serde(rename = "heroKey")]
    hero_key: i64,
    #[serde(rename = "HeroLevel", default)]
    level: i64,
    #[serde(rename = "IsUnLock", default)]
    unlocked: bool,
    /// Equipped items' `UniqueId`s (exact i64 — they exceed 2^53); 0 = empty slot.
    #[serde(rename = "equippedItemIds", default)]
    equipped: Vec<i64>,
}
#[derive(Deserialize)]
struct Slot {
    #[serde(rename = "ItemUniqueId")]
    item_unique_id: i64,
    /// How many of the item this slot holds — the game added stacking (materials stack up to 5). Older
    /// saves had no field (one item per slot), so a missing value defaults to 1.
    #[serde(rename = "Quantity", default = "one")]
    quantity: u64,
}

fn one() -> u64 {
    1
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
            *counts.entry(item_key).or_insert(0) += slot.quantity.max(1);
        }
    }

    let mut out: Vec<StashCount> = counts
        .into_iter()
        .map(|(item_key, count)| StashCount { item_key, count })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.item_key.cmp(&b.item_key)));
    out
}

/// Resolve each unlocked hero's equipped `UniqueId`s to items and group what's applied to each.
fn equipment(player: &PlayerSave) -> Vec<HeroGear> {
    let by_uid: HashMap<i64, &ItemInstance> = player.items.iter().map(|i| (i.unique_id, i)).collect();
    let mut heroes: Vec<HeroGear> = player
        .heroes
        .iter()
        .filter(|h| h.unlocked)
        .map(|h| HeroGear {
            hero_key: h.hero_key,
            level: h.level,
            items: h
                .equipped
                .iter()
                .filter_map(|uid| by_uid.get(uid)) // 0 / unknown = empty slot
                .map(|it| EquippedItem {
                    item_key: it.item_key,
                    materials: applied(it),
                })
                .collect(),
        })
        .filter(|h| !h.items.is_empty())
        .collect();
    heroes.sort_by_key(|h| h.hero_key);
    heroes
}

fn applied(item: &ItemInstance) -> Vec<StashCount> {
    let mut counts: HashMap<i64, u64> = HashMap::new();
    for e in item.enchants.iter().filter(|e| e.material_key != 0) {
        *counts.entry(e.material_key).or_insert(0) += 1;
    }
    let mut out: Vec<StashCount> = counts
        .into_iter()
        .map(|(item_key, count)| StashCount { item_key, count })
        .collect();
    out.sort_by_key(|m| m.item_key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap a PlayerSaveData object the way ES3 does (as a JSON string under `value`) and parse it.
    fn player(inner: &str) -> PlayerSave {
        let root = format!(r#"{{"PlayerSaveData":{{"value":{}}}}}"#, serde_json::to_string(inner).unwrap());
        parse_player(root.as_bytes()).expect("parses")
    }

    #[test]
    fn decrypts_and_aggregates_a_recorded_es3_save() {
        let bytes = include_bytes!("../../fixtures/es3-sample.bin");
        let json = decrypt_es3(bytes).expect("decrypts");
        let stash = aggregate(&parse_player(&json).expect("parses"));
        // 110001 appears twice (uids 1,2); 142002 once (uid 3); uid 99 has no instance → skipped.
        assert_eq!(stash.len(), 2);
        assert_eq!(stash[0].item_key, 110001);
        assert_eq!(stash[0].count, 2);
        assert!(stash.iter().any(|s| s.item_key == 142002 && s.count == 1));
    }

    #[test]
    fn sums_slot_quantity_for_stacks() {
        // Two stacks of the same material (qty 5 + 3) and a gear slot with no Quantity field (→ 1).
        let stash = aggregate(&player(
            r#"{
            "itemSaveDatas":[{"ItemKey":142002,"UniqueId":11},{"ItemKey":142002,"UniqueId":12},{"ItemKey":304011,"UniqueId":13}],
            "inventorySaveDatas":[{"ItemUniqueId":11,"Quantity":5}],
            "stashSaveDatas":[{"ItemUniqueId":12,"Quantity":3},{"ItemUniqueId":13}]
        }"#,
        ));
        assert_eq!(stash.iter().find(|s| s.item_key == 142002).unwrap().count, 8); // 5 + 3
        assert_eq!(stash.iter().find(|s| s.item_key == 304011).unwrap().count, 1); // no Quantity → 1
    }

    /// The gear-screen fields can't break the stash: a null/odd EnchantData or heroSaveDatas reads as empty.
    #[test]
    fn odd_gear_fields_never_break_the_stash() {
        let p = player(
            r#"{
            "itemSaveDatas":[{"ItemKey":142002,"UniqueId":11,"EnchantData":null}],
            "inventorySaveDatas":[{"ItemUniqueId":11,"Quantity":2}],
            "heroSaveDatas":{"unexpected":"shape"}
        }"#,
        );
        assert_eq!(aggregate(&p)[0].count, 2);
        assert!(equipment(&p).is_empty());
    }

    #[test]
    fn reads_equipped_gear_and_applied_materials() {
        // Ranger wears one item (uid is > 2^53, as in real saves) with two gems + one engraving applied and
        // an unused enchant entry; a locked hero and an empty slot (0) are ignored.
        let gear = equipment(&player(
            r#"{
            "itemSaveDatas":[{"ItemKey":316193,"UniqueId":516371047707024499,"EnchantData":[
                {"MaterialKey":116004},{"MaterialKey":116004},{"MaterialKey":124003},{"MaterialKey":0}]}],
            "heroSaveDatas":[
                {"heroKey":201,"HeroLevel":101,"IsUnLock":true,"equippedItemIds":[516371047707024499,0]},
                {"heroKey":301,"HeroLevel":1,"IsUnLock":false,"equippedItemIds":[516371047707024499]}]
        }"#,
        ));
        assert_eq!(gear.len(), 1);
        assert_eq!((gear[0].hero_key, gear[0].level), (201, 101));
        let item = &gear[0].items[0];
        assert_eq!(item.item_key, 316193);
        let mats: Vec<(i64, u64)> = item.materials.iter().map(|m| (m.item_key, m.count)).collect();
        assert_eq!(mats, vec![(116004, 2), (124003, 1)]);
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

    /// Caught mid-rotation the live file can be momentarily absent; the `.bak` still answers.
    #[test]
    fn falls_back_to_bak_when_live_is_briefly_missing() {
        let good = include_bytes!("../../fixtures/es3-sample.bin");
        let dir = std::env::temp_dir().join(format!("tbw-save-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("SaveFile_Live.es3");
        std::fs::write(dir.join("SaveFile_Live.es3.bak"), good).unwrap();

        let stash = SaveReader::with_path(live.clone()).read_stash().expect("falls back to .bak");
        assert_eq!(stash[0].item_key, 110001);

        std::fs::remove_dir_all(&dir).ok();
        // With neither file present it's still NotFound.
        assert!(matches!(SaveReader::with_path(live).read_stash(), Err(SaveError::NotFound)));
    }
}
