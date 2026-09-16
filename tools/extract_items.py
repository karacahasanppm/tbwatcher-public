#!/usr/bin/env python3
"""Regenerate src-tauri/src/save/item_map.json from the INSTALLED game — no third-party datamine.

The Sell Advisor bridges a save's numeric ItemKey to a Steam market_hash_name. Names + grades come
straight from the game's own Unity Localization "ItemTable" (always current, incl. items added by an
update); this replaces the old tbh-copilot dependency. Cosmetic-only fields the localization doesn't
carry (gear sprite paths, material fx-class/grade) are preserved from the previous item_map.json, so a
game update never regresses existing icons/tints; brand-new items simply get no icon until sprites are
added.

Run after a game update (needs UnityPy + the installed game):
    pip install UnityPy
    python tools/extract_items.py ["<TaskbarHero>/TaskBarHero_Data"]

How the bridge resolves an ItemKey at lookup (see save/mapping.rs), from the tables written here:
  - material (key band 1x) -> names[key] is the market_hash_name (plain).
  - gear -> base_id = key[0:2] + "00" + key[3:5]; name = names[base_id]; grade = GRADES[key[2]];
    market hash = "<name> (<grade>) <A|B|C>" (the game reuses A/B/C per line; the caller tries each).
    Verified live 2026-09: base-id formula matches every tradeable gear name.
"""
import io
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "src-tauri" / "src" / "save" / "item_map.json"
DEFAULT_GAME = r"C:/Program Files (x86)/Steam/steamapps/common/TaskbarHero/TaskBarHero_Data"

# Grade index -> English grade name (the ItemKey's 3rd digit; also the game's Grade_* localization order).
GRADES = ["Common", "Uncommon", "Rare", "Legendary", "Immortal",
          "Arcana", "Beyond", "Celestial", "Divine", "Cosmic"]


def base_id(key: int) -> int | None:
    """Gear base-item id (drops the grade digit + trailing variant) — the key under which the game
    localizes the item's name. Only defined for the 6-digit gear form (or the 6-digit core of a longer key)."""
    s = str(key)
    if len(s) < 6:
        return None
    return int(s[0:2] + "00" + s[3:5])


def load_game_names(data_dir: Path) -> dict[int, str]:
    """id -> English name, from the game's ItemTable localization (materials by full key, gear by base id)."""
    import UnityPy  # imported here so `--help`/import errors are obvious

    aa = data_dir / "StreamingAssets" / "aa" / "StandaloneWindows64"
    env = UnityPy.load(str(aa))
    shared, en = {}, {}
    for o in env.objects:
        if o.type.name != "MonoBehaviour":
            continue
        try:
            tt = o.read_typetree()
        except Exception:
            continue
        name = tt.get("m_Name", "")
        if name == "ItemTable Shared Data":
            for e in tt.get("m_Entries", []):
                shared[e["m_Id"]] = e.get("m_Key", "")
        elif name == "ItemTable_en-US":
            for e in tt.get("m_TableData", []):
                en[e["m_Id"]] = e.get("m_Localized", "")
    names: dict[int, str] = {}
    for mid, key in shared.items():
        if key.startswith("ItemName_") and mid in en:
            num = key[len("ItemName_"):]
            if num.isdigit() and en[mid]:
                names[int(num)] = en[mid]
    if not names:
        sys.exit("no ItemTable names found — is the game path right and UnityPy able to read it?")
    return names


def carry_cosmetics(prev: dict) -> tuple[dict[str, dict], dict[str, str]]:
    """Preserve the fields the localization doesn't carry, from the previous item_map.json — supporting
    both the old shape (material{}/gear{} keyed by full ItemKey) and this new shape."""
    materials: dict[str, dict] = {}
    gear_icons: dict[str, str] = {}
    if "materials" in prev and "names" in prev:  # already the new shape
        materials = prev.get("materials", {})
        gear_icons = prev.get("gearIcons", {})
        return materials, gear_icons
    # old shape
    for k, m in prev.get("material", {}).items():
        materials[k] = {"type": m.get("type", ""), "grade": m.get("grade", ""), "icon": m.get("icon", "")}
    for k, g in prev.get("gear", {}).items():  # gear icons are shared per base — key them by base id
        bid = base_id(int(k))
        if bid is not None and g.get("icon"):
            gear_icons.setdefault(str(bid), g["icon"])
    return materials, gear_icons


def main() -> None:
    data_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(DEFAULT_GAME)
    if not data_dir.exists():
        sys.exit(f"game data dir not found: {data_dir}")
    names = load_game_names(data_dir)
    prev = json.loads(OUT.read_text(encoding="utf-8")) if OUT.exists() else {}
    materials, gear_icons = carry_cosmetics(prev)

    out = {
        "names": {str(k): v for k, v in sorted(names.items())},
        "gearIcons": dict(sorted(gear_icons.items())),
        "materials": dict(sorted(materials.items())),
    }
    OUT.write_text(json.dumps(out, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    print(f"wrote {OUT}")
    print(f"  names={len(out['names'])}  gearIcons={len(out['gearIcons'])}  materials={len(out['materials'])}")


if __name__ == "__main__":
    main()
