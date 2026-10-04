<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { nav } from "$lib/nav.svelte";

  type Mover = {
    market_hash_name: string;
    old_price: string;
    lowest_price: string;
    volume: number | null;
    change_pct: number;
  };

  let movers = $state<Mover[]>([]);
  let loading = $state(true);
  // Start from a query another tab sent (e.g. the stash's "open in Movers"), consumed once.
  let query = $state(nav.moversQuery);
  nav.moversQuery = "";

  // "$1,003.09" → 100309 (USD only — the backend never serves another currency).
  const cents = (t: string) => Math.round(parseFloat(t.replace(/[^0-9.]/g, "")) * 100) || 0;

  // The whole market's 24h change, viewed through a sort lens — never a refetch.
  const SORTS = {
    move: { label: "biggest move", cmp: (a: Mover, b: Mover) => Math.abs(b.change_pct) - Math.abs(a.change_pct) },
    up: { label: "gainers", cmp: (a: Mover, b: Mover) => b.change_pct - a.change_pct },
    down: { label: "losers", cmp: (a: Mover, b: Mover) => a.change_pct - b.change_pct },
    price: { label: "price", cmp: (a: Mover, b: Mover) => cents(b.lowest_price) - cents(a.lowest_price) },
    name: { label: "name", cmp: (a: Mover, b: Mover) => a.market_hash_name.localeCompare(b.market_hash_name) },
  } as const;
  type Sort = keyof typeof SORTS;
  let sort = $state<Sort>("move");

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = q ? movers.filter((m) => m.market_hash_name.toLowerCase().includes(q)) : [...movers];
    return list.sort(SORTS[sort].cmp);
  });

  async function load() {
    loading = true;
    try {
      movers = await invoke<Mover[]>("movers_get");
    } finally {
      loading = false;
    }
  }
  onMount(load);
</script>

<div class="search">
  <input class="search-input" placeholder="search the market…" bind:value={query} spellcheck="false" />
  <button class="btn" onclick={load} disabled={loading}>{loading ? "…" : "refresh"}</button>
</div>

{#if loading}
  <p class="empty">reading the market…</p>
{:else if movers.length === 0}
  <p class="empty">No market data right now — try refresh in a bit.</p>
{:else}
  <div class="filters">
    {#each Object.entries(SORTS) as [key, s] (key)}
      <button class="chip" class:on={sort === key} onclick={() => (sort = key as Sort)}>{s.label}</button>
    {/each}
  </div>
  <div class="wl-head">
    <span>item ({shown.length})</span>
    <span class="mv-cols"><span>was → now</span><span>24h</span></span>
  </div>
  {#if shown.length === 0}
    <p class="empty">Nothing matches "{query.trim()}".</p>
  {/if}
  <ul class="list">
    {#each shown as m (m.market_hash_name)}
      <li class="row">
        <span class="name">{m.market_hash_name}</span>
        <span class="muted-price">{m.old_price}</span>
        <span class="arrow">→</span>
        <span class="price">{m.lowest_price}</span>
        <span class="change" class:up={m.change_pct > 0} class:down={m.change_pct < 0}>
          {m.change_pct > 0 ? "+" : ""}{m.change_pct}%
        </span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .mv-cols {
    display: flex;
    gap: 14px;
  }
  .arrow {
    color: var(--muted);
    font-size: 10px;
  }
  .change {
    font-variant-numeric: tabular-nums;
    min-width: 52px;
    text-align: right;
  }
  .change.up {
    color: #6bd08a;
  }
  .change.down {
    color: #e06a6a;
  }
</style>
