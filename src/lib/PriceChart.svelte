<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { usd } from "$lib/format";

  let { marketHashName }: { marketHashName: string } = $props();

  type Point = { t_ms: number; cents: number };

  const W = 240;
  const H = 72;
  const PAD = 5;

  let canvas = $state<HTMLCanvasElement | null>(null);
  let points = $state<Point[]>([]);
  let loading = $state(true);

  const cssVar = (n: string) =>
    getComputedStyle(document.documentElement).getPropertyValue(n).trim();

  function ago(ms: number): string {
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 90) return `${s}s`;
    const m = Math.round(s / 60);
    if (m < 90) return `${m}m`;
    const h = Math.round(m / 60);
    if (h < 36) return `${h}h`;
    return `${Math.round(h / 24)}d`;
  }

  // Price (y) and time (x) extents drive both the axis labels and the plot — the scale follows the data,
  // it is never fixed.
  const hi = $derived(points.length ? Math.max(...points.map((p) => p.cents)) : 0);
  const lo = $derived(points.length ? Math.min(...points.map((p) => p.cents)) : 0);
  const mid = $derived(Math.round((hi + lo) / 2));
  const t0 = $derived(points.length ? points[0].t_ms : 0);
  const tMid = $derived(points.length ? (t0 + points[points.length - 1].t_ms) / 2 : 0);

  // Crisp pixel line over faint axis guides; device-pixel scaled, smoothing off, integer coords (§4b).
  function draw() {
    const ctx = canvas?.getContext("2d");
    if (!ctx || !canvas || points.length < 2) return;

    const dpr = Math.max(1, Math.floor(window.devicePixelRatio || 1));
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.imageSmoothingEnabled = false;
    ctx.clearRect(0, 0, W, H);

    const cMin = lo;
    const cSpan = Math.max(1, hi - lo);
    const tMin = t0;
    const tSpan = Math.max(1, points[points.length - 1].t_ms - tMin);
    const x = (t: number) => Math.round(PAD + ((t - tMin) / tSpan) * (W - 2 * PAD));
    const y = (c: number) => Math.round(H - PAD - ((c - cMin) / cSpan) * (H - 2 * PAD));

    // Guide grid: horizontal at hi/mid/lo, vertical at the three time ticks the labels mark.
    ctx.strokeStyle = cssVar("--border") || "#4a3a63";
    ctx.lineWidth = 1;
    for (const c of [hi, mid, lo]) line(ctx, PAD, y(c), W - PAD, y(c));
    for (const gx of [PAD, Math.round(W / 2), W - PAD]) line(ctx, gx, PAD, gx, H - PAD);

    // Price line on top.
    ctx.strokeStyle = cssVar("--accent") || "#f2b134";
    ctx.lineWidth = 2;
    ctx.beginPath();
    points.forEach((p, i) =>
      i === 0 ? ctx.moveTo(x(p.t_ms), y(p.cents)) : ctx.lineTo(x(p.t_ms), y(p.cents)),
    );
    ctx.stroke();
  }

  function line(ctx: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number) {
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.stroke();
  }

  onMount(async () => {
    points = await invoke<Point[]>("price_history", { marketHashName });
    loading = false;
    requestAnimationFrame(draw);
  });
</script>

<div class="chart">
  {#if loading}
    <span class="chart-note">…</span>
  {:else if points.length < 2}
    <span class="chart-note">collecting price points — keep the app open to fill this in</span>
  {:else}
    <div class="chart-grid" style="--cw:{W}px;--ch:{H}px">
      <div class="y-axis">
        <span>{usd(hi)}</span>
        <span>{usd(mid)}</span>
        <span>{usd(lo)}</span>
      </div>
      <canvas bind:this={canvas} style="width:{W}px;height:{H}px"></canvas>
      <div class="x-axis">
        <span>{ago(t0)}</span>
        <span>{ago(tMid)}</span>
        <span>now</span>
      </div>
    </div>
  {/if}
</div>
