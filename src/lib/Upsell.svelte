<script lang="ts">
  import type { LimitHit } from "$lib/entitlement";

  // Respectful, dismissible upsell shown once when a metered limit is hit — no nag, no dark pattern
  // (INTENT §5.2, §7). The upgrade path arrives with the payment mechanism.
  let { hit, ondismiss }: { hit: LimitHit; ondismiss: () => void } = $props();
</script>

<div class="upsell">
  <div class="upsell-body">
    <span class="upsell-msg">{hit.message}</span>
    <span class="upsell-sub">Premium (coming soon) raises this limit.</span>
  </div>
  <button class="btn small" onclick={ondismiss}>got it</button>
</div>
