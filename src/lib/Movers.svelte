<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  type Mover = {
    market_hash_name: string;
    old_price: string;
    lowest_price: string;
    volume: number | null;
    change_pct: number;
  };

  let movers = $state<Mover[]>([]);
  let loading = $state(true);

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
  <span class="hint">biggest price moves across the market</span>
  <button class="btn" onclick={load} disabled={loading}>{loading ? "…" : "refresh"}</button>
</div>

{#if loading}
  <p class="empty">reading the market…</p>
{:else if movers.length === 0}
  <p class="empty">No moves yet — the tracker is still building a baseline. Check back soon.</p>
{:else}
  <div class="wl-head">
    <span>item</span>
    <span class="mv-cols"><span>was → now</span><span>change</span></span>
  </div>
  <ul class="list">
    {#each movers as m (m.market_hash_name)}
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
