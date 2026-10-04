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
    // The inventory gauge reads only the local save, so it goes first and shows up immediately.
    bag = await invoke<Bag>("inventory_status").catch(() => null);
    await invoke("watchlist_refresh").catch(() => {});
    await invoke("alerts_check").catch(() => {});
  }

  // In-game inventory fill: new loot is lost once it's full, so show how full it is and — from how fast it has
  // been filling while the app is open — roughly when it will be.
  type Bag = {
    used: number;
    open: number;
    stash_free: number;
    rate_per_hour: number | null;
    hours_to_full: number | null;
    error: string | null;
  };
  let bag = $state<Bag | null>(null);
  // Start the fill-rate measurement over (e.g. after switching stage) — the old rate no longer applies.
  const resetRate = async () => (bag = await invoke<Bag>("inventory_rate_reset").catch(() => bag));
  const eta = (hours: number) => {
    const min = Math.max(1, Math.round(hours * 60));
    const h = Math.floor(min / 60);
    const m = min % 60;
    return h === 0 ? `~${m}m` : m === 0 ? `~${h}h` : `~${h}h ${m}m`;
  };
  const bagNote = $derived.by(() => {
    if (!bag || bag.error) return "";
    if (bag.used >= bag.open) return "full";
    if (bag.rate_per_hour == null) return "measuring…";
    if (bag.hours_to_full == null) return "not filling";
    return `full in ${eta(bag.hours_to_full)}`;
  });

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
    {#if bag && !bag.error}
      <span
        class="bag"
        class:warn={bag.open > 0 && bag.used / bag.open >= 0.9}
        title={`In-game inventory slots in use — new loot lands here and is lost once it's full. Your stash has ${bag.stash_free} free slots: "Stash All" in-game moves the bag there.`}
      >
        bag {bag.used}/{bag.open} · {bagNote}
      </span>
      <button class="go" title="Restart the fill-rate measurement from now" onclick={resetRate}>↻</button>
    {/if}
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
