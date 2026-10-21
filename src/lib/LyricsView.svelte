<script lang="ts">
  // Paroles : synchronisées (ligne en cours mise en avant et gardée au centre si positionMs est
  // donné), sinon texte brut.
  import type { Lyrics } from "$lib/api";
  import { t } from "$lib/settings.svelte";

  let { lyrics, positionMs = null }: { lyrics: Lyrics; positionMs?: number | null } = $props();

  let box = $state<HTMLElement>();

  /** Index de la ligne en cours (dernière ligne commencée). */
  const current = $derived.by(() => {
    if (positionMs == null || !lyrics.lines.length) return -1;
    let lo = 0;
    let hi = lyrics.lines.length - 1;
    if (positionMs < lyrics.lines[0].ms) return -1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (lyrics.lines[mid].ms <= positionMs) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  });

  $effect(() => {
    if (current < 0 || !box) return;
    box.querySelector(`[data-i="${current}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
  });
</script>

{#if lyrics.lines.length}
  <div class="lines" class:live={positionMs != null} bind:this={box}>
    {#each lyrics.lines as l, i (i)}
      <p data-i={i} class:on={i === current} class:past={i < current}>{l.text || "♪"}</p>
    {/each}
  </div>
{:else if lyrics.plain}
  <div class="plain">{lyrics.plain}</div>
{:else if lyrics.instrumental}
  <p class="muted">{t("lyr.instrumental")}</p>
{:else}
  <p class="muted">{t("lyr.none")}</p>
{/if}

<style>
  .lines { display: grid; gap: 4px; }
  .lines p { margin: 0; color: var(--fg); }
  .live p { color: var(--fg-3); font-size: 18px; line-height: 1.4; transition: color 0.25s; }
  .live p.past { color: var(--fg-2); }
  .live p.on { color: var(--accent); font-weight: 600; }
  .plain { white-space: pre-line; color: var(--fg); }
</style>
