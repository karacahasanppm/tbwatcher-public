<script lang="ts">
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { mmss, asOf } from "$lib/format";

  type Entry = {
    market_hash_name: string;
    count: number;
    lowest_price: string | null;
    line_value_text: string | null;
    category: string;
  };
  type Advisor = {
    entries: Entry[];
    slot_limit: number;
    total_value_text: string | null;
    error: string | null;
    as_of_ms: number;
    next_scan_secs: number;
  };

  type Progress = { done: number; total: number };

  let advisor = $state<Advisor | null>(null);
  let loading = $state(false);
  let progress = $state<Progress | null>(null);
  // Seconds until a fresh rescan is allowed (Steam is only hit on a real scan); the button locks meanwhile.
  let cooldown = $state(0);
  let ticker: ReturnType<typeof setInterval> | undefined;

  // Stash filter (a view lens — never a re-scan). "all" plus the categories actually present, in a fixed
  // order so the chips don't reshuffle between scans.
  const CAT_ORDER = ["Wearable", "Decoration", "Engraving", "Inscription"];
  let filter = $state("all");
  const cats = $derived.by(() => {
    const present = new Set(advisor?.entries.map((e) => e.category));
    return CAT_ORDER.filter((c) => present.has(c));
  });
  const shown = $derived(
    !advisor ? [] : filter === "all" ? advisor.entries : advisor.entries.filter((e) => e.category === filter),
  );

  // The "spend a listing slot here" picks are the top `slot_limit` priced rows of the WHOLE stash — the
  // recommendation is global (you only have 4 slots), so it must not change when a category is filtered.
  // Precompute hash → 1-based rank from the full ranked list; the filtered view just looks items up.
  const pickRank = $derived.by(() => {
    const m = new Map<string, number>();
    if (!advisor) return m;
    for (const e of advisor.entries) {
      if (e.lowest_price != null && m.size < advisor.slot_limit) m.set(e.market_hash_name, m.size + 1);
    }
    return m;
  });

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
    if (loading || cooldown > 0) return;
    loading = true;
    progress = null;
    const onProgress = new Channel<Progress>();
    onProgress.onmessage = (p) => (progress = p);
    try {
      advisor = await invoke<Advisor>("stash_advisor", { onProgress });
      startCooldown(advisor.next_scan_secs);
    } finally {
      loading = false;
      progress = null;
    }
  }

  // Open the big game-themed stash grid in its own window (the visual companion to this ranked list).
  const openStash = () => invoke("open_stash_window").catch(() => {});

  onMount(load);
  onDestroy(() => clearInterval(ticker));
</script>

<div class="search">
  <span class="hint">reads your local save · prices are pre-fee</span>
  <button class="btn" onclick={openStash} disabled={loading || !advisor}>view stash</button>
  <button class="btn" onclick={load} disabled={loading || cooldown > 0}>
    {loading ? "…" : cooldown > 0 ? `rescan in ${mmss(cooldown)}` : "rescan"}
  </button>
</div>

{#if loading}
  <div class="loader">
    <span>
      {#if progress}valuing {progress.done}/{progress.total}{:else}reading save…{/if}
    </span>
    <div class="progress" class:indet={!progress}>
      <div
        class="progress-fill"
        style={progress && progress.total > 0
          ? `width:${(progress.done / progress.total) * 100}%`
          : ""}
      ></div>
    </div>
    <span class="dim">paced to stay polite to Steam</span>
  </div>
{:else if advisor?.error}
  <p class="empty">{advisor.error}</p>
{:else if advisor}
  <div class="wl-head">
    <span>stash ({advisor.entries.length}) · top {advisor.slot_limit} to list</span>
    <span class="total">{advisor.total_value_text ?? "—"}</span>
  </div>
  {#if advisor.as_of_ms > 0}
    <div class="as-of">
      data as of {asOf(advisor.as_of_ms)}{#if cooldown > 0} · rescan in {mmss(cooldown)}{/if}
    </div>
  {/if}
  {#if advisor.entries.length === 0}
    <p class="empty">No market-tradeable items in your stash.</p>
  {:else}
    {#if cats.length > 1}
      <div class="filters">
        <button class="chip" class:on={filter === "all"} onclick={() => (filter = "all")}>all</button>
        {#each cats as c (c)}
          <button class="chip" class:on={filter === c} onclick={() => (filter = c)}>{c}</button>
        {/each}
      </div>
    {/if}
    <ul class="list">
      {#each shown as e (e.market_hash_name)}
        {@const rank = pickRank.get(e.market_hash_name)}
        <li class="row" class:pick={rank != null}>
          <span class="slot">{rank ?? ""}</span>
          <span class="name">{e.market_hash_name}</span>
          <span class="vol">×{e.count}</span>
          <span class="muted-price">{e.lowest_price ?? "—"}</span>
          <span class="price">{e.line_value_text ?? "—"}</span>
        </li>
      {/each}
    </ul>
  {/if}
{/if}
