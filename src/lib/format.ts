// Shared display formatters, kept in one place so every view renders prices, times, and icons the same.

/** Steam economy icon URL for a description's `icon_url` hash, at the size the lists use. */
export const icon = (u: string) =>
  `https://community.fastly.steamstatic.com/economy/image/${u}/62fx62f`;

/** USD cents → "$1.23". */
export const usd = (cents: number) => `$${(cents / 100).toFixed(2)}`;

/** Whole seconds → "m:ss" for cooldown countdowns. */
export const mmss = (s: number) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;

/** Epoch ms → a compact "how long ago" label (absolute date once it's older than a day). */
export function asOf(ms: number): string {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return new Date(ms).toLocaleString();
}
