<script lang="ts">
  import { shownPath, writeTo } from "$lib/androidFiles";
  import { save } from "@tauri-apps/plugin-dialog";
  import { page } from "$app/state";
  import { api, entityHref, type Kind, type Mix, type MixFormat } from "$lib/api";
  import { duration, minutes, errText } from "$lib/format";
  import { lyricsLanguage } from "$lib/roles";
  import { MIX_SIZES, mixSizeOf, setMixSizeOf, settings, t } from "$lib/settings.svelte";
  import type { Key } from "$lib/locales/fr";
  import Cover from "$lib/Cover.svelte";

  let mixes = $state<Mix[] | null>(null);
  let error = $state<string | null>(null);
  let openKey = $state<string | null>(null);
  let message = $state<string | null>(null);
  let busy = $state(false);
  let sizing = $state(false);

  async function refresh() {
    try {
      const list = await api.mixes(settings.mixSize);
      mixes = list;
      // Mixes dont tu as choisi une autre taille : recalculés à cette taille.
      for (const m of list) {
        const n = mixSizeOf(m.key);
        if (n !== settings.mixSize) resize(m.key, n);
      }
    } catch (e) {
      error = errText(e);
    }
  }

  /** Recalcule un mix de la semaine à `n` titres et le remplace dans la liste. */
  async function resize(key: string, n: number) {
    const m = await api.mixAt(key, n);
    if (m && mixes) mixes = mixes.map((x) => (x.key === key ? m : x));
  }

  // Mix à la demande : /mixes?seed=artist&id=12 (depuis une fiche).
  const seed = $derived.by(() => {
    const kind = page.url.searchParams.get("seed") as Kind | null;
    const id = Number(page.url.searchParams.get("id"));
    return kind && id ? { kind, id } : null;
  });
  let seeded = $state<Mix | null>(null);
  const seedKey = $derived(seed ? `seed:${seed.kind}:${seed.id}` : null);
  /** Incrémenté pour recalculer le mix à la demande (taille changée). */
  let seedReload = $state(0);
  /** Pas assez de titres liés : nom du point de départ. */
  let seedEmpty = $state(false);
  let variant = $state(0);
  let seedLoading = $state(false);

  $effect(() => {
    if (seed) return;
    refresh();
  });

  $effect(() => {
    if (!seed) return;
    const v = variant;
    void seedReload;
    let alive = true;
    seedLoading = true;
    api
      .seededMix(seed.kind, seed.id, v, mixSizeOf(seedKey!))
      .then((m) => {
        if (!alive) return;
        seeded = m;
        seedEmpty = !m;
      })
      .catch((e) => alive && (error = errText(e)))
      .finally(() => alive && (seedLoading = false));
    return () => (alive = false);
  });

  const current = $derived(seed ? seeded : (mixes?.find((m) => m.key === openKey) ?? null));
  const cap = (s: string) => s.charAt(0).toLocaleUpperCase() + s.slice(1);

  function name(m: Mix): string {
    const p = m.params;
    switch (m.kind) {
      case "ambiance": {
        const [a, b] = p.traits.split(",").map((x) => t(`mx.trait.${x}` as Key));
        return b ? t("mx.name.ambiance2", { A: cap(a), b }) : t("mx.name.ambiance1", { a });
      }
      case "moment":
        return t(`mx.name.moment.${p.moment}` as Key);
      case "season":
        return t(`mx.name.season.${p.season}` as Key, { year: p.year });
      case "around":
        return t("mx.name.around", { artist: p.artist });
      case "seed":
        return t("mx.name.seed", { name: p.name });
      default:
        return t(`mx.name.${m.kind}` as Key);
    }
  }

  function why(m: Mix): string {
    const p = m.params;
    if (m.kind === "ambiance") {
      const lang = lyricsLanguage(p.language ?? null);
      return lang ? t("mx.why.ambianceLang", { lang }) : t("mx.why.ambiance");
    }
    if (m.kind === "rediscover") return t("mx.why.rediscover", { n: Math.round(Number(p.days) / 30) });
    if (m.kind === "around") return t("mx.why.around", { artist: p.artist });
    if (m.kind === "seed") return t(`mx.why.seed.${p.seed}` as Key);
    return t(`mx.why.${m.kind}` as Key);
  }

  /** Mosaïque de quatre pochettes, ou une seule si le mix en a moins. */
  const tiles = (m: Mix) => (m.covers.length < 4 ? m.covers.slice(0, 1) : m.covers);
  const albumName = (m: Mix, id: number) => m.tracks.find((x) => x.album_id === id)?.album ?? "";
  const count = (m: Mix) => t("mx.count", { n: m.tracks.length, d: minutes(m.duration_ms / 60000) });

  /** Retire un titre du mix affiché (et, pour « jamais », de tous les mixes à venir). */
  async function remove(m: Mix, trackId: number, never: boolean) {
    await api.mixFeedback(never ? "*" : m.key, trackId);
    m.tracks = m.tracks.filter((x) => x.id !== trackId);
    m.duration_ms = m.tracks.reduce((s, x) => s + (x.duration_ms ?? 210000), 0);
  }

  async function exportMix(m: Mix, format: MixFormat) {
    const title = name(m);
    const path = await save({
      defaultPath: `${title.replace(/[\\/:*?"<>|]+/g, " ").trim()}.${format}`,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    });
    if (!path) return;
    try {
      const file = `${title.replace(/[\\/:*?"<>|]+/g, " ").trim()}.${format}`;
      await writeTo(path, file, (p) => api.exportMix($state.snapshot(m), title, format, p));
      message = t("mx.exported", { path: shownPath(path, file) });
    } catch (e) {
      message = errText(e);
    }
  }

  async function resetFeedback() {
    if (busy) return;
    busy = true;
    try {
      const n = await api.clearMixFeedback();
      message = t("mx.feedbackDone", { n });
      await refresh();
    } finally {
      busy = false;
    }
  }

  function show(key: string | null) {
    openKey = key;
    message = null;
    window.scrollTo({ top: 0 });
  }
</script>

<div class="page">
  {#if seed && seedEmpty}
    <button class="link back" onclick={() => history.back()}>← {t("mx.backSeed")}</button>
    <div class="panel empty">{t("mx.seedEmpty")}</div>
  {:else if current}
    {@const m = current}
    <header class="mixhead">
      {#if seed}
        <button class="link back" onclick={() => history.back()}>← {t("mx.backSeed")}</button>
      {:else}
        <button class="link back" onclick={() => show(null)}>← {t("mx.back")}</button>
      {/if}
      <div class="hero">
        <div class="mosaic big" class:single={m.covers.length < 4} aria-hidden="true">
          {#each tiles(m) as id (id)}<Cover kind="album" {id} name={albumName(m, id)} size={86} />{/each}
        </div>
        <div class="heroText">
          <div class="eyebrow">{t("nav.mixes")}</div>
          <h1>{name(m)}</h1>
          <p class="why">{why(m)}</p>
          <p class="muted small">{count(m)}</p>
        </div>
      </div>
      <div class="exports">
        <span class="muted small">{t("mx.export")}</span>
        {#each ["xspf", "jspf", "csv"] as const as f (f)}
          <button class="btn" title={t(`mx.fmt.${f}`)} onclick={() => exportMix(m, f)}>{f.toUpperCase()}</button>
        {/each}
        {#if seed}
          <button class="btn" disabled={seedLoading} onclick={() => variant++}>{t("mx.reroll")}</button>
        {/if}
        <label class="size">
          <span class="muted small">{t("mx.size")}</span>
          <select
            value={String(mixSizeOf(seedKey ?? m.key))}
            onchange={async (e) => {
              const n = Number(e.currentTarget.value);
              setMixSizeOf(seedKey ?? m.key, n);
              if (seed) seedReload++;
              else {
                sizing = true;
                await resize(m.key, n).finally(() => (sizing = false));
              }
            }}
          >
            {#each MIX_SIZES as n (n)}<option value={n}>{t("mx.sizeN", { n: Number(n) })}</option>{/each}
          </select>
        </label>
      </div>
      {#if message}<p class="muted small msg">{message}</p>{/if}
    </header>

    <section class="panel" class:dim={seedLoading || sizing}>
      <ol class="tracks">
        {#each m.tracks as tr, i (tr.id)}
          <li>
            <span class="mono muted n">{i + 1}</span>
            {#if tr.album_id}
              <Cover kind="album" id={tr.album_id} name={tr.album ?? tr.title} size={36} />
            {:else}
              <Cover kind="track" id={tr.id} name={tr.title} size={36} remote={false} />
            {/if}
            <span class="what">
              <a href={entityHref("track", tr.id)}>{tr.title}</a>
              <span class="muted small">
                <a class="sub" href={entityHref("artist", tr.artist_id)}>{tr.artist}</a>{#if tr.album && tr.album_id}{" · "}<a class="sub" href={entityHref("album", tr.album_id)}>{tr.album}</a>{/if}
              </span>
            </span>
            <span class="mono muted small">{t("mx.plays", { n: tr.plays })}</span>
            <span class="mono muted small len">{tr.duration_ms ? duration(tr.duration_ms / 1000) : ""}</span>
            <span class="actions">
              <button class="icon" title={t("mx.remove")} aria-label={t("mx.remove")} onclick={() => remove(m, tr.id, false)}>✕</button>
              <button class="icon" title={t("mx.never")} aria-label={t("mx.never")} onclick={() => remove(m, tr.id, true)}>⊘</button>
            </span>
          </li>
        {/each}
      </ol>
    </section>
  {:else}
    <header>
      <div class="eyebrow">{t("nav.mixes")}</div>
      <h1>{t("mx.title")}</h1>
      <p class="intro">{t("mx.intro")}</p>
    </header>

    {#if error}
      <p class="muted">{error}</p>
    {:else if mixes && !mixes.length}
      <div class="panel empty">{t("mx.empty")}</div>
    {:else if mixes}
      <div class="grid">
        {#each mixes as m (m.key)}
          <button class="card" onclick={() => show(m.key)}>
            <span class="mosaic" class:single={m.covers.length < 4} aria-hidden="true">
              {#each tiles(m) as id (id)}<Cover kind="album" {id} name={albumName(m, id)} size={70} />{/each}
            </span>
            <span class="cardText">
              <span class="cardTitle">{name(m)}</span>
              <span class="cardWhy">{why(m)}</span>
              <span class="muted small">{count(m)}</span>
            </span>
          </button>
        {/each}
      </div>
      {#if !mixes.some((m) => m.kind === "ambiance")}
        <p class="muted small">{t("mx.hintFeatures")}</p>
      {/if}
      <p class="muted small">
        <button class="link" disabled={busy} onclick={resetFeedback}>{t("mx.feedback")}</button>
        {#if message}· {message}{/if}
      </p>
    {/if}
  {/if}
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 1100px; }
  .dim { opacity: 0.55; transition: opacity 0.2s; }
  header { display: grid; gap: 6px; }
  .intro { margin: 0; max-width: 75ch; color: var(--fg-2); }
  p { margin: 0; }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 14px; }
  /* Téléphone : deux mixes par ligne. */
  @media (max-width: 900px) {
    :global(.android) .grid { grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 10px; }
    :global(.android) .card { padding: 8px; gap: 8px; }
    :global(.android) .cardTitle { font-size: 15px; }
  }
  .card {
    display: grid;
    gap: 10px;
    text-align: left;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 12px;
    color: var(--fg);
    align-content: start;
  }
  .card:hover { border-color: var(--fg-3); }
  .mosaic {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-radius: 7px;
    overflow: hidden;
    aspect-ratio: 1;
    background: var(--chip);
  }
  .mosaic.single { grid-template-columns: 1fr; }
  .mosaic :global(.cover) { width: 100%; height: 100%; border-radius: 0; }
  .mosaic :global(.cover img) { width: 100%; height: 100%; }
  .cardText { display: grid; gap: 3px; }
  .cardTitle { font-family: var(--font-display); font-size: 16px; font-weight: 500; }
  .cardWhy { font-size: 13px; color: var(--fg-2); }

  .mixhead { gap: 14px; }
  .back { justify-self: start; }
  .hero { display: flex; gap: 20px; align-items: end; flex-wrap: wrap; }
  .mosaic.big { width: 172px; flex: none; }
  .heroText { display: grid; gap: 6px; min-width: 0; flex: 1 1 280px; }
  .why { color: var(--fg-2); max-width: 70ch; }
  .exports { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .exports .btn { padding: 4px 12px; font-size: 13px; }
  .size { display: flex; gap: 6px; align-items: center; margin-left: auto; }
  .size select { font: inherit; font-size: 13px; color: var(--fg); background: var(--surface); border: 1px solid var(--line); border-radius: 6px; padding: 4px 6px; }

  .tracks { list-style: none; margin: 0; padding: 0; display: grid; }
  .tracks li {
    display: grid;
    grid-template-columns: 2.2em auto minmax(0, 1fr) auto auto auto;
    gap: 4px 12px;
    align-items: center;
    padding: 6px 0;
    border-bottom: 1px solid var(--line);
  }
  .tracks li:last-child { border-bottom: 0; }
  .n { text-align: right; }
  .len { min-width: 3.2em; text-align: right; }
  .what { display: grid; min-width: 0; }
  .what > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { color: inherit; }
  .actions { display: flex; gap: 2px; opacity: 0.35; }
  .tracks li:hover .actions, .actions:focus-within { opacity: 1; }
  .icon { background: none; border: 0; color: var(--fg-2); padding: 2px 6px; border-radius: 5px; }
  .icon:hover { background: var(--chip); color: var(--fg); }
  .link { background: none; border: 0; color: var(--accent); padding: 0; font-size: inherit; }
  .link:hover { text-decoration: underline; }
  @media (max-width: 700px) {
    .tracks li { grid-template-columns: 2.2em auto minmax(0, 1fr) auto; }
    .tracks li > .mono:nth-of-type(2), .tracks li > .mono:nth-of-type(3) { display: none; }
  }
</style>
