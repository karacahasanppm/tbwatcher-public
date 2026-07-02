// Regenerates item_map.json — the ItemKey -> Steam market_hash_name bridge for the Sell Advisor.
// Not compiled (Node, run by hand on a game update). Run:
//   node item_map.gen.cjs <path-to-tbh-copilot-clone>
//
// Source data: github.com/shigake/tbh-copilot (MIT-licensed CODE; the bundled game tables —
// gamedata.js / gearnames.js / materialfx.js — are TBH's own content, used here only to bridge a
// save the player already owns to Steam's public market identifiers). See mapping.rs for attribution.
//
// Output shape: { material: {key: {name, type, icon, grade}}, gear: {key: {name, grade, icon}} }
//  - material name is already the English market_hash_name; type is the fx class
//    (Decoration | Engraving | Inscription) that drives the Sell Advisor's stash filter.
//  - icon is the game's own sprite path ("/game/...") — the visual stash grid renders it from the images
//    copied into static/game/ (attribution below). Regenerating here does NOT copy the PNGs; refresh those
//    from <clone>/assets/game/ separately when the game updates.
//  - gear hash is built at lookup as "<name> (<grade>) A" (verified live), falling back to "<name> (<grade>)".
//  - only grades tradeable on Steam now: Legendary, Immortal, Arcana, Beyond
//    (Celestial/Divine/Cosmic exist but are not market-tradeable yet).
const fs = require('fs'), path = require('path');
const CP = process.argv[2];
if (!CP) { console.error('usage: node item_map.gen.cjs <tbh-copilot-clone>'); process.exit(1); }
const DB = require(CP + '/engine/gamedata.js');
const GN = require(CP + '/engine/gearnames.js');
const MF = require(CP + '/engine/materialfx.js');
const TRADE_NOW = new Set(['Legendary', 'Immortal', 'Arcana', 'Beyond']);
const norm = gr => gr ? gr.charAt(0) + gr.slice(1).toLowerCase() : gr;

const material = {};
for (const m of MF) {
  const it = DB.items[String(m.key)];
  material[String(m.key)] = { name: m.name, type: m.type, icon: it && it.icon, grade: m.grade };
}

const gear = {};
for (const k in GN) {
  const it = DB.items[String(k)];
  if (!it || !it.grade) continue;
  const g = norm(it.grade);
  if (TRADE_NOW.has(g)) gear[String(k)] = { name: GN[k], grade: g, icon: it.icon };
}

fs.writeFileSync(path.join(__dirname, 'item_map.json'), JSON.stringify({ material, gear }));
console.log('materials:', Object.keys(material).length, '| gear keys:', Object.keys(gear).length);
