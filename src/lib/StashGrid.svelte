<script lang="ts">
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { onMount } from "svelte";
  import { asOf } from "$lib/format";

  type Entry = {
    market_hash_name: string;
    count: number;
    lowest_price: string | null;
    line_value_text: string | null;
    category: string;
    icon: string;
    grade: string;
  };

  // Game-style rarity colours (from the datamined TBH grade palette). Slots are tinted by grade, like the
  // in-game inventory. Keyed by upper-case grade; unknown/empty falls back to the neutral border.
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
  const gradeColor = (g: string) => GRADE_COL[g?.toUpperCase()] ?? "var(--border)";
  type Advisor = {
    entries: Entry[];
    slot_limit: number;
    total_value_text: string | null;
    error: string | null;
    as_of_ms: number;
    next_scan_secs: number;
    limit: unknown;
  };
  type Progress = { done: number; total: number };

  let advisor = $state<Advisor | null>(null);
  let loading = $state(true);
  let progress = $state<Progress | null>(null);

  // The grid rides the same governed scan as the Sell Advisor list (cached within the cooldown), so opening
  // this window costs no extra Steam load beyond a fresh scan.
  async function load() {
    loading = true;
    progress = null;
    const onProgress = new Channel<Progress>();
    onProgress.onmessage = (p) => (progress = p);
    try {
      advisor = await invoke<Advisor>("stash_advisor", { onProgress });
    } finally {
      loading = false;
    }
  }
  // The window lives hidden from startup (defined in tauri.conf), so don't scan on mount — scan when it's
  // actually shown/focused. In dev/browser (no Tauri window) just scan directly.
  onMount(async () => {
    try {
      const w = getCurrentWebviewWindow();
      await w.onFocusChanged(({ payload: focused }) => {
        if (focused) load();
      });
      if (await w.isVisible()) load();
    } catch {
      load();
    }
  });
</script>

<div class="stash-win">
  <header class="stash-head">
    <h1 class="wordmark">stash</h1>
    {#if advisor && !loading}<span class="total">{advisor.total_value_text ?? "—"}</span>{/if}
  </header>

  {#if loading}
    <div class="loader">
      <span>{#if progress}valuing {progress.done}/{progress.total}{:else}reading save…{/if}</span>
      <div class="progress" class:indet={!progress}>
        <div
          class="progress-fill"
          style={progress && progress.total > 0 ? `width:${(progress.done / progress.total) * 100}%` : ""}
        ></div>
      </div>
      <span class="dim">paced to stay polite to Steam</span>
    </div>
  {:else if advisor?.error}
    <p class="empty">{advisor.error}</p>
  {:else if advisor && advisor.entries.length}
    <div class="as-of">data as of {asOf(advisor.as_of_ms)} · prices pre-fee</div>
    <div class="grid">
      {#each advisor.entries as e (e.market_hash_name)}
        {@const col = gradeColor(e.grade)}
        <div class="cell" title="{e.market_hash_name} · {e.grade}">
          <div
            class="slot"
            style={col.startsWith("#") ? `border-color:${col};box-shadow:inset 0 0 10px ${col}2b` : ""}
          >
            <img class="sprite" src={e.icon} alt={e.market_hash_name} />
            {#if e.count > 1}<span class="qty">×{e.count}</span>{/if}
          </div>
          <span class="cell-price">{e.lowest_price ?? "—"}</span>
        </div>
      {/each}
    </div>
  {:else}
    <p class="empty">No market-tradeable items in your stash.</p>
  {/if}
</div>

<style>
  /* The stash window: a game-inventory-style slot grid, on-brand pixel art (CLAUDE.md §4b), not a clone
     of the game's UI. Slots hold the item's own sprite; the price sits under each, like a market ledger. */
  .stash-win {
    padding: 14px 16px;
    min-height: 100vh;
    box-sizing: border-box;
  }
  .stash-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    border-bottom: 2px solid var(--border);
    padding-bottom: 8px;
    margin-bottom: 10px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(84px, 1fr));
    gap: 8px;
  }
  .cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .slot {
    position: relative;
    width: 100%;
    aspect-ratio: 1;
    background: var(--bg);
    border: 2px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .sprite {
    width: 72%;
    height: 72%;
    object-fit: contain;
    image-rendering: pixelated;
  }
  /* Stack count, bottom-right like an inventory slot. */
  .qty {
    position: absolute;
    right: 2px;
    bottom: 1px;
    font-size: 10px;
    color: var(--ink);
    background: var(--shadow);
    padding: 0 3px;
    border: 1px solid var(--border);
  }
  .cell-price {
    font-size: 11px;
    color: var(--accent);
    font-weight: bold;
  }
</style>
