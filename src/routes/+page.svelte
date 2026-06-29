<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import MarketWatch from "$lib/MarketWatch.svelte";
  import Portfolio from "$lib/Portfolio.svelte";
  import Alerts from "$lib/Alerts.svelte";
  import SellAdvisor from "$lib/SellAdvisor.svelte";

  type Tab = "market" | "portfolio" | "alerts" | "stash";
  let tab = $state<Tab>("market");

  const subtitle: Record<Tab, string> = {
    market: "market watch",
    portfolio: "portfolio",
    alerts: "alerts",
    stash: "stash & sell advisor",
  };

  // The price watcher runs from the shell so it keeps going on every tab (and while minimized to tray):
  // prices stay fresh, alerts fire, and the watchlist price history keeps accumulating regardless of which
  // tab is open. One paced cadence drives both — alerts_check reads the cache watchlist_refresh just filled.
  const POLL_MS = 180_000;
  let pollTimer: ReturnType<typeof setInterval>;
  async function poll() {
    await invoke("watchlist_refresh").catch(() => {});
    await invoke("alerts_check").catch(() => {});
  }
  onMount(() => {
    poll();
    pollTimer = setInterval(poll, POLL_MS);
  });
  onDestroy(() => clearInterval(pollTimer));
</script>

<main class="app">
  <header class="bar">
    <h1 class="wordmark">tbwatcher</h1>
    <span class="sub">{subtitle[tab]}</span>
  </header>

  <nav class="tabs">
    <button class="tab" class:active={tab === "market"} onclick={() => (tab = "market")}>
      market
    </button>
    <button class="tab" class:active={tab === "portfolio"} onclick={() => (tab = "portfolio")}>
      portfolio
    </button>
    <button class="tab" class:active={tab === "alerts"} onclick={() => (tab = "alerts")}>
      alerts
    </button>
    <button class="tab" class:active={tab === "stash"} onclick={() => (tab = "stash")}>
      stash
    </button>
  </nav>

  {#if tab === "market"}
    <MarketWatch />
  {:else if tab === "portfolio"}
    <Portfolio />
  {:else if tab === "alerts"}
    <Alerts />
  {:else}
    <SellAdvisor />
  {/if}
</main>
