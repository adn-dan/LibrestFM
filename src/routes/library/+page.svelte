<script lang="ts">
  // Bibliothèque : tout ce qui a été écouté, trié et filtré, chargé par pages.
  import { api, entityHref, type BrowsePage, type BrowseSort, type Kind } from "$lib/api";
  import Cover from "$lib/Cover.svelte";
  import { date, hours, num, errText } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  const PAGE = 100;
  let kind = $state<Kind>("artist");
  let sort = $state<BrowseSort>("plays");
  let desc = $state(true);
  let filter = $state("");
  /** Filtre appliqué (après une courte pause de frappe). */
  let applied = $state("");
  let page = $state<BrowsePage | null>(null);
  /** Pochettes déjà en cache des lignes affichées (pas de téléchargement ici). */
  let covers = $state<Record<number, string>>({});
  const loadCovers = (k: Kind, p: BrowsePage) =>
    api.cachedImages(k, p.rows.map((r) => r.item.id)).then((m) => (covers = { ...covers, ...m })).catch(() => {});
  let loading = $state(false);
  let error = $state("");

  // Recherche relancée 200 ms après la dernière frappe.
  $effect(() => {
    const f = filter;
    const id = setTimeout(() => (applied = f.trim()), 200);
    return () => clearTimeout(id);
  });

  $effect(() => {
    const args = [kind, sort, desc, applied] as const;
    let alive = true;
    loading = true;
    api
      .browse(...args, 0, PAGE)
      .then((p) => {
        if (!alive) return;
        page = p;
        error = "";
        covers = {};
        loadCovers(args[0], p);
      })
      .catch((e) => alive && (error = errText(e)))
      .finally(() => alive && (loading = false));
    return () => (alive = false);
  });

  async function more() {
    if (!page || loading) return;
    loading = true;
    try {
      const next = await api.browse(kind, sort, desc, applied, page.rows.length, PAGE);
      page = { total: next.total, rows: [...page.rows, ...next.rows] };
      loadCovers(kind, next);
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  /** Clic sur un en-tête : trie par cette colonne, ou inverse le sens. */
  function sortBy(s: BrowseSort) {
    if (sort === s) desc = !desc;
    else {
      sort = s;
      desc = s !== "name";
    }
  }
  const arrow = (s: BrowseSort) => (sort === s ? (desc ? " ↓" : " ↑") : "");
  const KINDS: Kind[] = ["artist", "album", "track"];
</script>

{#snippet head(s: BrowseSort, label: string, cls = "")}
  <th class={cls} aria-sort={sort === s ? (desc ? "descending" : "ascending") : "none"}>
    <button onclick={() => sortBy(s)}>{label}{arrow(s)}</button>
  </th>
{/snippet}

<div class="page">
  <header>
    <div class="eyebrow">{t("nav.library")}</div>
    <h1>{t("lib.title")}</h1>
  </header>

  <div class="bar">
    <div class="seg" role="group" aria-label={t("lib.kind")}>
      {#each KINDS as k (k)}
        <button aria-pressed={kind === k} onclick={() => (kind = k)}>{t(`kinds.${k}`)}</button>
      {/each}
    </div>
    <input type="search" bind:value={filter} placeholder={t("lib.filter")} aria-label={t("lib.filter")} />
    {#if page}<span class="muted small">{t("lib.count", { n: page.total })}</span>{/if}
  </div>

  {#if error}
    <p class="panel" role="alert">{t("error", { msg: error })}</p>
  {:else if page}
    <section class="panel" class:dim={loading}>
      <table>
        <thead>
          <tr>
            <th class="n">#</th>
            {@render head("name", t(`kinds.${kind}`), "name")}
            {@render head("plays", t("kpi.plays"), "num")}
            {@render head("time", t("kpi.listened"), "num")}
            {@render head("first", t("kpi.first"), "num hide")}
            {@render head("last", t("kpi.last"), "num hide")}
          </tr>
        </thead>
        <tbody>
          {#each page.rows as r, i (r.item.id)}
            <tr>
              <td class="n mono muted">{i + 1}</td>
              <td class="name">
                <span class="who">
                  <Cover kind={r.item.kind} id={r.item.id} name={r.item.name} size={32} cached={covers[r.item.id] ?? null} />
                  <span class="txt">
                    <a href={entityHref(r.item.kind, r.item.id)}>{r.item.name}</a>
                    {#if r.item.artist && r.item.artist_id}<a class="sub small" href={entityHref("artist", r.item.artist_id)}>{r.item.artist}</a>{/if}
                  </span>
                </span>
              </td>
              <td class="num mono">{num(r.item.plays)}</td>
              <td class="num mono">{hours(r.seconds)}</td>
              <td class="num mono muted hide">{date(r.first_ts)}</td>
              <td class="num mono muted hide">{date(r.last_ts)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if !page.rows.length}<p class="muted">{t("lib.none")}</p>{/if}
      {#if page.rows.length < page.total}
        <button class="btn more" onclick={more} disabled={loading}>{t("lib.more", { n: Math.min(PAGE, page.total - page.rows.length) })}</button>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 1200px; }
  header { display: grid; gap: 6px; }
  .bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .bar input { font: inherit; flex: 1 1 240px; max-width: 420px; color: var(--fg); background: var(--surface); border: 1px solid var(--line); border-radius: 7px; padding: 6px 10px; }
  .dim { opacity: 0.6; transition: opacity 0.2s; }
  table { width: 100%; border-collapse: collapse; }
  th, td { padding: 6px 10px 6px 0; border-bottom: 1px solid var(--line); text-align: left; }
  th button { background: none; border: 0; padding: 0; font: inherit; font-weight: 500; color: var(--fg-2); cursor: pointer; }
  th[aria-sort="ascending"] button, th[aria-sort="descending"] button { color: var(--fg); }
  .num { text-align: right; white-space: nowrap; }
  th.num button { text-align: right; }
  .n { width: 3em; text-align: right; }
  .name { width: 100%; max-width: 0; }
  .who { display: flex; gap: 10px; align-items: center; min-width: 0; }
  .txt { display: grid; min-width: 0; }
  .txt a { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { color: var(--fg-2); }
  .more { margin-top: 12px; }
  tbody tr:last-child td { border-bottom: 0; }
  @media (max-width: 760px) {
    .hide { display: none; }
  }
  /* Téléphone : noms sur deux lignes plutôt que coupés après quelques lettres, sans le rang. */
  @media (max-width: 480px) {
    :global(.android) .n { display: none; }
    :global(.android) .txt a { white-space: normal; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  }
</style>
