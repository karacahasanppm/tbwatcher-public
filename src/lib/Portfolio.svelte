<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { icon, mmss, asOf } from "$lib/format";

  type Holding = { market_hash_name: string; name: string; icon_url: string; count: number };
  type Entry = { holding: Holding; lowest_price: string | null; value_text: string | null };
  type Portfolio = {
    steam_id: string | null;
    entries: Entry[];
    total_value_text: string | null;
    error: string | null;
    as_of_ms: number;
    next_reload_secs: number;
    stale: boolean;
  };

  let steamId = $state("");
  let portfolio = $state<Portfolio | null>(null);
  let loading = $state(false);
  // Reload cooldown (server-enforced & persisted); the UI just counts it down for feedback.
  let cooldown = $state(0);
  let ticker: ReturnType<typeof setInterval> | undefined;

  // Lock the load button while the current account is on cooldown — but unlock the moment the user edits
  // the SteamID, so switching accounts is never blocked.
  const locked = $derived(cooldown > 0 && portfolio?.steam_id === steamId);
  function startCooldown(secs: number) {
    clearInterval(ticker);
    cooldown = secs;
    if (secs > 0) {
      ticker = setInterval(() => {
        if (--cooldown <= 0) {
          cooldown = 0;
          clearInterval(ticker);
        }
      }, 1000);
    }
  }

  async function load() {
    loading = true;
    try {
      portfolio = await invoke<Portfolio>("portfolio_get");
      startCooldown(portfolio.next_reload_secs);
    } catch (e) {
      // Never blank out on a rejected command — surface it.
      portfolio = {
        steam_id: steamId || null,
        entries: [],
        total_value_text: null,
        error: String(e),
        as_of_ms: 0,
        next_reload_secs: 0,
        stale: true,
      };
    } finally {
      loading = false;
    }
  }
  async function save(e: Event) {
    e.preventDefault();
    await invoke("steamid_set", { steamId });
    await load();
  }

  onMount(async () => {
    steamId = (await invoke<string | null>("steamid_get")) ?? "";
    if (steamId) load();
  });
  onDestroy(() => clearInterval(ticker));
</script>

<form class="search" onsubmit={save}>
  <input
    class="search-input"
    placeholder="SteamID64…"
    bind:value={steamId}
    spellcheck="false"
  />
  <button class="btn" type="submit" disabled={loading || locked}>
    {loading ? "…" : locked ? `wait ${mmss(cooldown)}` : "load"}
  </button>
</form>

{#if loading && !portfolio}
  <p class="empty">loading…</p>
{:else if portfolio}
  {#if portfolio.error && portfolio.entries.length === 0}
    <p class="empty">{portfolio.error}</p>
  {:else}
    {#if portfolio.error}
      <p class="warn">{portfolio.error}</p>
    {/if}
    <div class="wl-head">
      <span>holdings ({portfolio.entries.length})</span>
      <span class="total">{portfolio.total_value_text ?? "—"}</span>
    </div>
    {#if portfolio.as_of_ms > 0}
      <div class="as-of">
        data as of {asOf(portfolio.as_of_ms)}{#if cooldown > 0} · next refresh in {mmss(cooldown)}{/if}
      </div>
    {/if}
    {#if portfolio.entries.length === 0}
      <p class="empty">No marketable items found.</p>
    {:else}
      <ul class="list">
        {#each portfolio.entries as { holding, lowest_price, value_text } (holding.market_hash_name)}
          <li class="row">
            <img class="ico" src={icon(holding.icon_url)} alt="" />
            <span class="name">{holding.name}</span>
            <span class="vol">×{holding.count}</span>
            <span class="muted-price">{lowest_price ?? "—"}</span>
            <span class="price">{value_text ?? "—"}</span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
{/if}
