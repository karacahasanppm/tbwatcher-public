<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import PriceChart from "$lib/PriceChart.svelte";
  import { icon } from "$lib/format";
  import { getConfig } from "$lib/config";

  type CatalogItem = {
    market_hash_name: string;
    name: string;
    icon_url: string;
    item_type: string;
    name_color: string | null;
  };
  type Quote = {
    market_hash_name: string;
    lowest_price: string | null;
    median_price: string | null;
    volume: number | null;
    fetched_at_ms: number;
  };
  type WatchEntry = { item: CatalogItem; quote: Quote | null };

  // The shell does the Steam polling app-wide; the Market view just re-reads the cache to reflect it,
  // at a config-driven cadence (backend-swappable).

  let watchlist = $state<WatchEntry[]>([]);
  let results = $state<CatalogItem[]>([]);
  let query = $state("");
  let searching = $state(false);
  let refreshing = $state(false);
  let now = $state(Date.now());

  const watched = $derived(new Set(watchlist.map((e) => e.item.market_hash_name)));
  // One expanded price chart at a time; toggling re-mounts PriceChart so it pulls the latest series.
  let open = $state<string | null>(null);
  const toggle = (h: string) => (open = open === h ? null : h);

  async function loadWatchlist() {
    watchlist = await invoke<WatchEntry[]>("watchlist_get");
  }
  async function refreshWatchlist() {
    refreshing = true;
    try {
      watchlist = await invoke<WatchEntry[]>("watchlist_refresh");
    } finally {
      refreshing = false;
    }
  }
  async function search(e: Event) {
    e.preventDefault();
    if (!query.trim()) return;
    searching = true;
    try {
      results = await invoke<CatalogItem[]>("catalog_search", { query });
    } finally {
      searching = false;
    }
  }
  async function add(item: CatalogItem) {
    await invoke("watchlist_add", { item });
    await refreshWatchlist();
  }
  async function remove(market_hash_name: string) {
    await invoke("watchlist_remove", { marketHashName: market_hash_name });
    await loadWatchlist();
  }

  const ageSec = (ms: number) => Math.max(0, Math.floor((now - ms) / 1000));

  let syncId: ReturnType<typeof setInterval>;
  let tickId: ReturnType<typeof setInterval>;
  onMount(async () => {
    await loadWatchlist(); // cached quotes the shell keeps fresh — no Steam hit on tab open
    const cfg = await getConfig();
    syncId = setInterval(loadWatchlist, cfg.sync_ms);
    tickId = setInterval(() => (now = Date.now()), 1_000);
  });
  onDestroy(() => {
    clearInterval(syncId);
    clearInterval(tickId);
  });
</script>

<form class="search" onsubmit={search}>
  <input
    class="search-input"
    placeholder="search items…"
    bind:value={query}
    spellcheck="false"
  />
  <button class="btn" type="submit" disabled={searching}>
    {searching ? "…" : "find"}
  </button>
</form>

{#if results.length}
  <ul class="list results">
    {#each results as item (item.market_hash_name)}
      <li class="row">
        <img class="ico" src={icon(item.icon_url)} alt="" />
        <span class="name">{item.name}</span>
        <button
          class="btn small"
          class:active={open === item.market_hash_name}
          title="price history"
          onclick={() => toggle(item.market_hash_name)}>~</button
        >
        <button
          class="btn small"
          onclick={() => add(item)}
          disabled={watched.has(item.market_hash_name)}
        >
          {watched.has(item.market_hash_name) ? "✓" : "+"}
        </button>
      </li>
      {#if open === item.market_hash_name}
        <li class="chart-row"><PriceChart marketHashName={item.market_hash_name} /></li>
      {/if}
    {/each}
  </ul>
{/if}

<div class="wl-head">
  <span>watchlist ({watchlist.length})</span>
  {#if refreshing}<span>refreshing…</span>{/if}
</div>

{#if watchlist.length === 0}
  <p class="empty">No items yet. Search above and add some.</p>
{:else}
  <ul class="list">
    {#each watchlist as { item, quote } (item.market_hash_name)}
      <li class="row">
        <img class="ico" src={icon(item.icon_url)} alt="" />
        <span class="name">{item.name}</span>
        <span class="price">{quote?.lowest_price ?? "—"}</span>
        <span class="vol">{quote?.volume?.toLocaleString() ?? "—"}</span>
        <span class="age">{quote ? `${ageSec(quote.fetched_at_ms)}s` : ""}</span>
        <button
          class="btn small"
          class:active={open === item.market_hash_name}
          title="price history"
          onclick={() => toggle(item.market_hash_name)}>~</button
        >
        <button class="btn small" onclick={() => remove(item.market_hash_name)}>✕</button>
      </li>
      {#if open === item.market_hash_name}
        <li class="chart-row"><PriceChart marketHashName={item.market_hash_name} /></li>
      {/if}
    {/each}
  </ul>
{/if}
