<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount, onDestroy } from "svelte";
  import { getConfig } from "$lib/config";
  import MarketWatch from "$lib/MarketWatch.svelte";
  import Movers from "$lib/Movers.svelte";
  import Portfolio from "$lib/Portfolio.svelte";
  import Alerts from "$lib/Alerts.svelte";
  import SellAdvisor from "$lib/SellAdvisor.svelte";

  type Tab = "market" | "movers" | "portfolio" | "alerts" | "stash";
  let tab = $state<Tab>("market");

  const subtitle: Record<Tab, string> = {
    market: "market watch",
    movers: "market movers",
    portfolio: "portfolio",
    alerts: "alerts",
    stash: "stash & sell advisor",
  };

  // The price watcher runs from the shell so it keeps going on every tab (and while minimized to tray):
  // prices stay fresh, alerts fire, and the watchlist price history keeps accumulating regardless of which
  // tab is open. One paced cadence drives both — alerts_check reads the cache watchlist_refresh just filled.
  // The cadence is config-driven (backend-swappable), fetched once at startup.
  let pollTimer: ReturnType<typeof setInterval>;
  async function poll() {
    await invoke("watchlist_refresh").catch(() => {});
    await invoke("alerts_check").catch(() => {});
  }

  // tbwatcher is fully free with no limits; a single, unobtrusive support link is the whole monetization.
  const DONATE_URL = "https://www.patreon.com/karacahasan";
  const donate = () => DONATE_URL && openUrl(DONATE_URL);

  // Backend-driven "update available" note (dormant until the backend is wired up — returns null otherwise).
  type Update = { version: string; notes?: string | null; url?: string | null };
  let update = $state<Update | null>(null);

  onMount(async () => {
    poll();
    invoke<Update | null>("backend_version_check").then((u) => (update = u)).catch(() => {});
    const cfg = await getConfig();
    pollTimer = setInterval(poll, cfg.poll_ms);
  });
  onDestroy(() => clearInterval(pollTimer));
</script>

<main class="app">
  <button class="donate" onclick={donate}>
    <span class="donate-heart">♥</span>
    <span>free &amp; unlimited — if it helps you trade, you can support development</span>
    <span class="donate-sub">donate ↗</span>
  </button>

  {#if update}
    <button class="update-note" onclick={() => update?.url && openUrl(update.url)}>
      update available — v{update.version}{#if update.url} · download ↗{/if}
    </button>
  {/if}

  <header class="bar">
    <h1 class="wordmark">tbwatcher</h1>
    <span class="sub">{subtitle[tab]}</span>
  </header>

  <nav class="tabs">
    <button class="tab" class:active={tab === "market"} onclick={() => (tab = "market")}>
      market
    </button>
    <button class="tab" class:active={tab === "movers"} onclick={() => (tab = "movers")}>
      movers
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
  {:else if tab === "movers"}
    <Movers />
  {:else if tab === "portfolio"}
    <Portfolio />
  {:else if tab === "alerts"}
    <Alerts />
  {:else}
    <SellAdvisor />
  {/if}
</main>
