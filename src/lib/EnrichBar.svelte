<script lang="ts">
  // Barre de répartition des titres : trouvés, introuvables, erreurs ; le reste est en attente.
  import type { JobStatus } from "./enrich.svelte";
  import { shares } from "./enrich.svelte";
  import { t } from "./settings.svelte";

  let { st, height = 10 }: { st: JobStatus; height?: number } = $props();
  const s = $derived(shares(st));
</script>

<div class="progress" style:height={`${height}px`} role="img" aria-label={t("en.barAria", { p: Math.round(s.ok) })}>
  <i class="ok" style:width={`${s.ok}%`}></i>
  <i class="nf" style:width={`${s.notFound}%`}></i>
  <i class="er" style:width={`${s.errors}%`}></i>
</div>

<style>
  .progress { display: flex; border-radius: 999px; overflow: hidden; background: var(--chip); }
  i { display: block; height: 100%; transition: width 0.4s ease; }
  .ok { background: var(--s3); }
  .nf { background: var(--s2); }
  .er { background: var(--danger); }
</style>
