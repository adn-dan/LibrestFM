<script lang="ts">
  // Ce que l'artiste a fait chez d'autres artistes écoutés (producteur, musicien, auteur…),
  // d'après les crédits MusicBrainz où il figure comme personne.
  import { api, entityHref, type CreditedFor } from "./api";
  import Cover from "./Cover.svelte";
  import { roleFromPair } from "./roles";
  import { t } from "./settings.svelte";

  let { id, name }: { id: number; name: string } = $props();
  let list = $state<CreditedFor[] | null>(null);

  $effect(() => {
    let alive = true;
    list = null;
    api
      .artistCredits(id)
      .then((r) => alive && (list = r))
      .catch(() => {});
    return () => (alive = false);
  });

  const roles = (r: string[]) => [...new Set(r.map(roleFromPair))].join(", ");
</script>

{#if list?.length}
  <section class="panel">
    <h2>{t("acr.title")}</h2>
    <p class="muted small">{t("acr.help", { name })}</p>
    <ul class="list">
      {#each list as c (c.artist.id)}
        <li>
          <Cover kind="artist" id={c.artist.id} name={c.artist.name} size={36} />
          <div class="body">
            <div class="head">
              <a href={entityHref("artist", c.artist.id)}>{c.artist.name}</a>
              <span class="muted small">{roles(c.roles)}</span>
            </div>
            <div class="tracks small">
              {#each c.tracks.slice(0, 6) as [tid, title, plays], i (tid)}<a href={entityHref("track", tid)}>{title}</a> <span class="mono muted">{plays}×</span>{#if i < Math.min(6, c.tracks.length) - 1}{" · "}{/if}{/each}
              {#if c.tracks.length > 6}<span class="muted">{" "}{t("acr.more", { n: c.tracks.length - 6 })}</span>{/if}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  p { margin: 0; }
  .list { list-style: none; margin: 0; padding: 0; display: grid; gap: 12px; }
  .list li { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 12px; align-items: start; }
  .body { display: grid; gap: 2px; min-width: 0; }
  .head { display: flex; flex-wrap: wrap; gap: 4px 10px; align-items: baseline; }
  .tracks { color: var(--fg-2); line-height: 1.5; }
</style>
