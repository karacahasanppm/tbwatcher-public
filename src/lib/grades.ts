// Game-style rarity colours (from the datamined TBH grade palette), keyed by upper-case grade. Used to tint
// items by grade the way the in-game inventory does; unknown/empty falls back to the neutral border.
const GRADE_COL: Record<string, string> = {
  COMMON: "#9aa7c2",
  UNCOMMON: "#74d28e",
  RARE: "#5fd0e0",
  LEGENDARY: "#f6c552",
  IMMORTAL: "#ff8a5c",
  ARCANA: "#a98cff",
  BEYOND: "#ff6fae",
  CELESTIAL: "#6fe0d0",
  DIVINE: "#ffd76a",
  COSMIC: "#ff5fae",
};

export const gradeColor = (g: string) => GRADE_COL[g?.toUpperCase()] ?? "var(--border)";
