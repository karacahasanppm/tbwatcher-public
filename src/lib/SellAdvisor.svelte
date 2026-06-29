<script lang="ts">
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { mmss, asOf } from "$lib/format";
  import Upsell from "$lib/Upsell.svelte";
  import type { LimitHit } from "$lib/entitlement";

  type Entry = {
    market_hash_name: string;
    count: number;
    lowest_price: string | null;
    line_value_text: string | null;
  };
  type Advisor = {
    entries: Entry[];
    slot_limit: number;
    total_value_text: string | null;
    error: string | null;
    as_of_ms: number;
    next_scan_secs: number;
    limit: LimitHit | null;
  };

  type Progress = { done: number; total: number };

  let advisor = $state<Advisor | null>(null);
  let loading = $state(false);
  let progress = $state<Progress | null>(null);
  // Once-at-limit upsell when the trial capped the scan (stash_max); dismissible, re-set on each scan.
  let upsell = $state<LimitHit | null>(null);
  // Seconds until a fresh rescan is allowed (Steam is only hit on a real scan); the button locks meanwhile.
  let cooldown = $state(0);
  let ticker: ReturnType<typeof setInterval> | undefined;

  // A pick = one of the top `slot_limit` priced rows; spend a scarce listing slot here first.
  const isPick = (e: Entry, i: number) =>
    advisor != null && i < advisor.slot_limit && e.lowest_price != null;

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
      upsell = advisor.limit;
      startCooldown(advisor.next_scan_secs);
    } finally {
      loading = false;
      progress = null;
    }
  }

  onMount(load);
  onDestroy(() => clearInterval(ticker));
</script>

<div class="search">
  <span class="hint">reads your local save · prices are pre-fee</span>
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
  {#if upsell}
    <Upsell hit={upsell} ondismiss={() => (upsell = null)} />
  {/if}
  {#if advisor.entries.length === 0}
    <p class="empty">No market-tradeable items in your stash.</p>
  {:else}
    <ul class="list">
      {#each advisor.entries as e, i (e.market_hash_name)}
        <li class="row" class:pick={isPick(e, i)}>
          <span class="slot">{isPick(e, i) ? i + 1 : ""}</span>
          <span class="name">{e.market_hash_name}</span>
          <span class="vol">×{e.count}</span>
          <span class="muted-price">{e.lowest_price ?? "—"}</span>
          <span class="price">{e.line_value_text ?? "—"}</span>
        </li>
      {/each}
    </ul>
  {/if}
{/if}
