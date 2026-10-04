<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { asOf } from "$lib/format";
  import { gradeColor } from "$lib/grades";

  type Material = { name: string; category: string; icon: string; count: number; price: string | null };
  type Gear = {
    name: string;
    grade: string;
    icon: string;
    tradeable: boolean;
    price: string | null;
    materials: Material[];
    materials_value_text: string | null;
  };
  type Hero = {
    hero_key: number;
    name: string;
    level: number;
    items: Gear[];
    gear_value_text: string;
    materials_value_text: string;
  };
  type View = {
    heroes: Hero[];
    gear_value_text: string;
    materials_value_text: string;
    error: string | null;
    as_of_ms: number;
  };

  let view = $state<View | null>(null);
  let loading = $state(false);

  async function load() {
    if (loading) return;
    loading = true;
    try {
      view = await invoke<View>("equipment_get");
    } finally {
      loading = false;
    }
  }
  onMount(load);
</script>

<div class="search">
  <span class="hint">what your heroes are wearing · reads your local save</span>
  <button class="btn" onclick={load} disabled={loading}>{loading ? "…" : "refresh"}</button>
</div>

{#if loading && !view}
  <p class="empty">reading save…</p>
{:else if view?.error}
  <p class="empty">{view.error}</p>
{:else if view && view.heroes.length === 0}
  <p class="empty">No equipped gear found in your save.</p>
{:else if view}
  <div class="wl-head">
    <span>gear {view.gear_value_text}</span>
    <span class="total">applied {view.materials_value_text}</span>
  </div>
  <div class="as-of">
    gear = its market listing · applied gems, engravings &amp; scrolls = what they'd cost to buy again
    (they stay on the item) · pre-fee · as of {asOf(view.as_of_ms)}
  </div>

  {#each view.heroes as h (h.hero_key)}
    <div class="hero-head">
      <span class="hero-name">{h.name}</span>
      <span class="dim">Lv {h.level}</span>
      <span class="hero-sum">{h.gear_value_text} · applied {h.materials_value_text}</span>
    </div>
    <ul class="list">
      {#each h.items as g, i (i)}
        <li class="gear" style={`border-left-color:${gradeColor(g.grade)}`}>
          <div class="row">
            {#if g.icon}<img class="ico" src={g.icon} alt="" />{:else}<span class="ico"></span>{/if}
            <span class="name">{g.name} <span class="dim">({g.grade})</span></span>
            {#if g.tradeable}
              <span class="price">{g.price ?? "—"}</span>
            {:else}
              <span class="muted-price">not tradeable</span>
            {/if}
          </div>
          {#if g.materials.length > 0}
            <div class="mats">
              {#each g.materials as m (m.name)}
                <span class="mat">{m.count}× {m.name} <span class="dim">{m.price ?? "—"}</span></span>
              {/each}
              <span class="mat-sum">{g.materials_value_text}</span>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/each}
{/if}

<style>
  .hero-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 14px 0 4px;
    font-size: 12px;
  }
  .hero-name {
    color: var(--ink);
    font-weight: bold;
  }
  .hero-sum {
    margin-left: auto;
    color: var(--accent);
    font-size: 11px;
  }
  .dim {
    color: var(--muted);
  }
  /* Grade-tinted edge, like the stash grid's slot tint. */
  .gear {
    border-left: 3px solid var(--border);
    padding-left: 4px;
    border-bottom: 1px solid var(--border);
  }
  .gear:last-child {
    border-bottom: none;
  }
  .gear .row {
    border-bottom: none;
  }
  .mats {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    padding: 0 2px 6px 32px;
    font-size: 10px;
    color: var(--ink);
  }
  .mat-sum {
    margin-left: auto;
    color: var(--accent);
  }
</style>
