<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { usd } from "$lib/format";

  type CatalogItem = {
    market_hash_name: string;
    name: string;
    icon_url: string;
    item_type: string;
    name_color: string | null;
  };
  type WatchEntry = { item: CatalogItem; quote: unknown };
  type Alert = {
    market_hash_name: string;
    name: string;
    target_cents: number;
    direction: "below" | "above";
    triggered: boolean;
  };

  let items = $state<CatalogItem[]>([]);
  let alerts = $state<Alert[]>([]);
  let selected = $state("");
  let target = $state("");
  let direction = $state<"below" | "above">("below");

  async function loadAlerts() {
    alerts = await invoke<Alert[]>("alerts_list");
  }
  async function loadItems() {
    const wl = await invoke<WatchEntry[]>("watchlist_get");
    items = wl.map((e) => e.item);
    if (!selected && items.length) selected = items[0].market_hash_name;
  }
  async function addAlert(e: Event) {
    e.preventDefault();
    const item = items.find((i) => i.market_hash_name === selected);
    if (!item || !target.trim()) return;
    await invoke("alerts_set", {
      marketHashName: item.market_hash_name,
      name: item.name,
      targetPrice: target,
      direction,
    });
    target = "";
    await loadAlerts();
    invoke("alerts_check").catch(() => {}); // fire immediately if the threshold is already crossed
  }
  async function removeAlert(a: Alert) {
    await invoke("alerts_remove", { marketHashName: a.market_hash_name, direction: a.direction });
    await loadAlerts();
  }

  onMount(() => {
    loadItems();
    loadAlerts();
  });
</script>

{#if items.length === 0}
  <p class="empty">Add items to your watchlist first — alerts watch watchlist prices.</p>
{:else}
  <form class="search alert-form" onsubmit={addAlert}>
    <select class="search-input" bind:value={selected}>
      {#each items as i (i.market_hash_name)}
        <option value={i.market_hash_name}>{i.name}</option>
      {/each}
    </select>
    <select class="search-input dir" bind:value={direction}>
      <option value="below">≤</option>
      <option value="above">≥</option>
    </select>
    <input class="search-input price-in" placeholder="$0.00" bind:value={target} />
    <button class="btn" type="submit">add</button>
  </form>
{/if}

<div class="wl-head">
  <span>alerts ({alerts.length})</span>
</div>

{#if alerts.length === 0}
  <p class="empty">No alerts set.</p>
{:else}
  <ul class="list">
    {#each alerts as a (a.market_hash_name + a.direction)}
      <li class="row">
        <span class="name">{a.name}</span>
        <span class="muted-price">{a.direction === "below" ? "≤" : "≥"} {usd(a.target_cents)}</span>
        <span class="age">{a.triggered ? "✓" : ""}</span>
        <button class="btn small" onclick={() => removeAlert(a)}>✕</button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  /* The alert form has four controls; let it wrap in the narrow window. */
  .alert-form {
    flex-wrap: wrap;
  }
  .dir {
    flex: 0 0 52px;
  }
  .price-in {
    flex: 0 0 80px;
  }
</style>
