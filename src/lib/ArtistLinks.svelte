<script lang="ts">
  // Artistes reliés par des personnes créditées chez les deux (membres, producteurs, musiciens…).
  import { api, entityHref, personHref, type ArtistLink } from "./api";
  import Cover from "./Cover.svelte";
  import { num } from "./format";
  import { roleFromPair } from "./roles";
  import { t } from "./settings.svelte";

  let { id, name }: { id: number; name: string } = $props();
  let links = $state<ArtistLink[] | null>(null);

  $effect(() => {
    let alive = true;
    links = null;
    api
      .artistLinks(id)
      .then((r) => alive && (links = r))
      .catch(() => {});
    return () => (alive = false);
  });

  /** Trois rôles au plus, sans doublon une fois traduits. */
  const roles = (list: string[]) => [...new Set(list.map(roleFromPair))].slice(0, 3).join(", ");
</script>

{#if links?.length}
  <section class="panel">
    <h2>{t("links.title")}</h2>
    <p class="muted small">{t("links.help", { name })}</p>
    <ul class="links">
      {#each links as l (l.artist.id)}
        <li>
          <Cover kind="artist" id={l.artist.id} name={l.artist.name} size={36} />
          <div class="body">
            <div class="head">
              <a href={entityHref("artist", l.artist.id)}>{l.artist.name}</a>
              <span class="mono muted small">{t("plays", { n: l.artist.plays })}</span>
            </div>
            <div class="people small">
              {#each l.people as p, i (p.id)}<a href={personHref(p.id, p.artist_id)}>{p.name}</a> <span class="muted">({roles(p.roles)})</span>{#if i < l.people.length - 1}{" · "}{/if}{/each}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  p { margin: 0; }
  .links { list-style: none; margin: 0; padding: 0; display: grid; gap: 12px; }
  .links li { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 12px; align-items: start; }
  .body { display: grid; gap: 2px; min-width: 0; }
  .head { display: flex; gap: 10px; align-items: baseline; }
  .people { color: var(--fg-2); line-height: 1.5; }
</style>
