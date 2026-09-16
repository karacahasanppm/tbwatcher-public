<script lang="ts">
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import MainShell from "$lib/MainShell.svelte";
  import StashGrid from "$lib/StashGrid.svelte";

  // One SPA, two views. The stash window loads the app root and injects `window.__TBW_STASH__` before the
  // app boots — the reliable signal (independent of routing/label). `?view=stash` is the browser/dev
  // fallback; the window label is a last resort for a Tauri window opened without the injected flag.
  function detectStash(): boolean {
    if ((window as unknown as { __TBW_STASH__?: boolean }).__TBW_STASH__ === true) return true;
    if (new URLSearchParams(location.search).get("view") === "stash") return true;
    try {
      return getCurrentWebviewWindow().label === "stash";
    } catch {
      return false;
    }
  }
  const isStash = detectStash();
</script>

{#if isStash}
  <StashGrid />
{:else}
  <MainShell />
{/if}
