<script lang="ts">
  // Panneau « Paroles » de la page d'un titre : cherchées à l'ouverture (gardées ensuite).
  import { api, type Lyrics } from "$lib/api";
  import LyricsView from "$lib/LyricsView.svelte";
  import { t } from "$lib/settings.svelte";

  let { artist, title, album = null, durationMs = null }: { artist: string; title: string; album?: string | null; durationMs?: number | null } =
    $props();

  let lyrics = $state<Lyrics | null>(null);
  let failed = $state(false);

  $effect(() => {
    const key = `${artist}\u0001${title}`;
    lyrics = null;
    failed = false;
    api
      .lyrics(artist, title, album, durationMs)
      .then((l) => key === `${artist}\u0001${title}` && (lyrics = l))
      .catch(() => (failed = true));
  });
</script>

<section class="panel">
  <h2>{t("lyr.title")}</h2>
  {#if lyrics}
    <LyricsView {lyrics} />
  {:else if failed}
    <p class="muted">{t("lyr.error")}</p>
  {:else}
    <p class="muted">{t("lyr.loading")}</p>
  {/if}
</section>

<style>
  p { margin: 0; }
</style>
