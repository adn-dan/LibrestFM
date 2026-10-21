<script lang="ts">
  // « Fusionner avec… » : choisir la fiche (même type) qui regroupera celle-ci.
  import { goto } from "$app/navigation";
  import { api, entityHref, type Kind, type Ranked } from "./api";
  import { t } from "./settings.svelte";

  let { kind, id, name }: { kind: Kind; id: number; name: string } = $props();

  let open = $state(false);
  let query = $state("");
  let hits = $state<Ranked[]>([]);
  let target = $state<Ranked | null>(null);
  let busy = $state(false);
  let timer: ReturnType<typeof setTimeout>;

  function search() {
    clearTimeout(timer);
    target = null;
    const q = query.trim();
    if (q.length < 2) return (hits = []);
    timer = setTimeout(async () => {
      const r = await api.search(q).catch(() => null);
      const list = r ? (kind === "artist" ? r.artists : kind === "album" ? r.albums : r.tracks) : [];
      hits = list.filter((h) => h.id !== id);
    }, 200);
  }

  /** Aucune fiche à ce nom : celle-ci est renommée (fusionnée dans une fiche créée à ce nom). */
  async function rename() {
    const name = query.trim();
    if (!name || busy) return;
    busy = true;
    try {
      goto(entityHref(kind, await api.renameEntity(kind, id, name)));
    } finally {
      busy = false;
    }
  }

  async function confirm() {
    if (!target || busy) return;
    busy = true;
    try {
      await api.mergeEntities(kind, id, target.id);
      goto(entityHref(kind, target.id));
    } finally {
      busy = false;
    }
  }
</script>

<div class="merge">
  {#if !open}
    <button class="btn small" onclick={() => (open = true)}>{t("mg.merge")}</button>
  {:else}
    <label class="sr" for="merge-q">{t("mg.merge")}</label>
    <input id="merge-q" type="search" placeholder={t("mg.placeholder")} bind:value={query} oninput={search} autocomplete="off" />
    {#if target}
      <div class="confirm">
        <span>{t("mg.confirm", { from: name, to: target.name })}</span>
        <button class="btn primary" disabled={busy} onclick={confirm}>{t("mg.do")}</button>
        <button class="btn" onclick={() => (target = null)}>{t("src.cancel")}</button>
      </div>
    {:else if hits.length}
      <ul>
        {#each hits as h (h.id)}
          <li>
            <button class="hit" onclick={() => (target = h)}>
              <span>{h.name}</span>
              {#if h.artist}<span class="muted small">{h.artist}</span>{/if}
              <span class="mono muted small">{t("plays", { n: h.plays })}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if !target && query.trim().length >= 2 && !hits.some((h) => h.name.toLowerCase() === query.trim().toLowerCase())}
      <button class="btn" disabled={busy} onclick={rename}>{t("mg.rename", { name: query.trim() })}</button>
    {/if}
    <p class="muted small">{t("mg.help")}</p>
  {/if}
</div>

<style>
  .merge { display: grid; gap: 6px; max-width: 520px; }
  .btn.small { justify-self: start; padding: 4px 10px; font-size: 13px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
  input { font: inherit; color: var(--fg); background: var(--surface); border: 1px solid var(--line); border-radius: 7px; padding: 6px 10px; }
  ul { list-style: none; margin: 0; padding: 4px; display: grid; gap: 2px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); max-height: 260px; overflow-y: auto; }
  .hit { display: flex; gap: 10px; align-items: baseline; width: 100%; text-align: left; background: none; border: 0; border-radius: 6px; padding: 5px 8px; color: var(--fg); }
  .hit:hover { background: var(--chip); }
  .hit .mono { margin-left: auto; }
  .confirm { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  p { margin: 0; }
</style>
