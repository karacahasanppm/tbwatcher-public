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
  import Equipment from "$lib/Equipment.svelte";
  import { nav, type Tab } from "$lib/nav.svelte";

  const subtitle: Record<Tab, string> = {
    market: "market watch",
    movers: "market movers",
    portfolio: "portfolio",
    alerts: "alerts",
    stash: "stash & sell advisor",
    gear: "equipped gear",
  };

  // The price watcher runs from the shell so it keeps going on every tab:
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
    <span class="sub">{subtitle[nav.tab]}</span>
  </header>

  <nav class="tabs">
    <button class="tab" class:active={nav.tab === "market"} onclick={() => (nav.tab = "market")}>
      market
    </button>
    <button class="tab" class:active={nav.tab === "movers"} onclick={() => (nav.tab = "movers")}>
      movers
    </button>
    <button class="tab" class:active={nav.tab === "portfolio"} onclick={() => (nav.tab = "portfolio")}>
      portfolio
    </button>
    <button class="tab" class:active={nav.tab === "alerts"} onclick={() => (nav.tab = "alerts")}>
      alerts
    </button>
    <button class="tab" class:active={nav.tab === "stash"} onclick={() => (nav.tab = "stash")}>
      stash
    </button>
    <button class="tab" class:active={nav.tab === "gear"} onclick={() => (nav.tab = "gear")}>
      gear
    </button>
  </nav>

  {#if nav.tab === "market"}
    <MarketWatch />
  {:else if nav.tab === "movers"}
    <Movers />
  {:else if nav.tab === "portfolio"}
    <Portfolio />
  {:else if nav.tab === "alerts"}
    <Alerts />
  {:else if nav.tab === "stash"}
    <SellAdvisor />
  {:else}
    <Equipment />
  {/if}
</main>
