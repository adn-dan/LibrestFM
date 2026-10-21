<script lang="ts">
  // Découvertes : titres jamais écoutés, trouvés à partir des artistes les plus
  // écoutés (ou d'un seul, depuis sa fiche). Extraits de 30 s, avis, export.
  import { shownPath, writeTo } from "$lib/androidFiles";
  import { page } from "$app/state";
  import { listen } from "@tauri-apps/api/event";
  import { save } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, type DiscoverScope, type DiscoverStatus, type Found, type Mix, type MixFormat, type Selection } from "$lib/api";
  import { date, duration, errText } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  const artistId = $derived(Number(page.url.searchParams.get("artist")) || null);
  const artistName = $derived(page.url.searchParams.get("name") ?? "");
  const scope = $derived<DiscoverScope>(artistId ? { kind: "artist", id: artistId } : { kind: "weekly" });

  let sel = $state<Selection | null>(null);
  let status = $state<DiscoverStatus | null>(null);
  let running = $state(false);
  let error = $state("");
  let message = $state("");
  /** Titres aimés pendant cette visite (surlignés). */
  let liked = $state<Set<number>>(new Set());
  /** Extrait en cours de lecture. */
  let playing = $state<number | null>(null);
  let audio: HTMLAudioElement;

  const load = async (s: DiscoverScope) => {
    try {
      sel = await api.discoverSelection(s);
    } catch (e) {
      error = errText(e);
    }
  };

  $effect(() => {
    const s = scope;
    sel = null;
    load(s);
    api.discoverRunning().then((r) => (running = r));
  });

  // Sélection autour d'un artiste jamais faite : lancée tout de suite, une seule
  // fois par artiste (un arrêt ou une erreur ne la relance pas).
  const tried = new Set<number>();
  $effect(() => {
    if (artistId && sel && !sel.computed_at && !running && !tried.has(artistId)) {
      tried.add(artistId);
      start();
    }
  });

  $effect(() => {
    const off = listen<DiscoverStatus>("discover-progress", (e) => {
      status = e.payload;
      running = e.payload.running;
      if (!e.payload.running) load(scope);
    });
    const offErr = listen<string>("discover-error", (e) => {
      error = e.payload;
      running = false;
    });
    return () => {
      off.then((f) => f());
      offErr.then((f) => f());
    };
  });

  async function start() {
    error = "";
    running = true;
    status = null;
    await api.startDiscover(scope);
  }

  async function play(f: Found) {
    if (playing === f.deezer_id) {
      audio.pause();
      playing = null;
      return;
    }
    const url = await api.discoverPreview(f.deezer_id).catch(() => null);
    if (!url) {
      message = t("dc.noPreview");
      return;
    }
    audio.src = url;
    await audio.play().catch(() => {});
    playing = f.deezer_id;
  }

  async function verdict(f: Found, v: number) {
    await api.discoverVerdict(f.deezer_id, f.artist, v);
    if (v < 0 && sel) {
      if (playing === f.deezer_id) audio.pause();
      sel = {
        ...sel,
        new_artists: sel.new_artists.filter((x) => x.deezer_id !== f.deezer_id),
        known_artists: sel.known_artists.filter((x) => x.deezer_id !== f.deezer_id),
      };
    } else {
      const next = new Set(liked);
      if (v > 0) next.add(f.deezer_id);
      else next.delete(f.deezer_id);
      liked = next;
    }
  }

  /** Les deux listes en une playlist, au format des mixes. */
  async function exportAll(format: MixFormat) {
    if (!sel) return;
    const title = artistId ? t("dc.titleArtist", { name: artistName }) : t("dc.title");
    const all = [...sel.new_artists, ...sel.known_artists];
    const mix: Mix = {
      key: "discover",
      kind: "seed",
      params: {},
      covers: [],
      duration_ms: all.reduce((s, f) => s + (f.duration_s ?? 210) * 1000, 0),
      tracks: all.map((f) => ({ id: f.deezer_id, title: f.title, artist: f.artist, artist_id: 0, album_id: null, album: f.album, plays: 0, duration_ms: f.duration_s ? f.duration_s * 1000 : null })),
    };
    const path = await save({ defaultPath: `${title.replace(/[\\/:*?"<>|]+/g, " ").trim()}.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
    if (!path) return;
    const file = `${title.replace(/[\\/:*?"<>|]+/g, " ").trim()}.${format}`;
    await writeTo(path, file, (p) => api.exportMix(mix, title, format, p))
      .then(() => (message = t("mx.exported", { path: shownPath(path, file) })))
      .catch((e) => (message = errText(e)));
  }

  const why = (f: Found) => (f.because.length ? t("dc.because", { names: f.because.join(", ") }) : "");
</script>

<audio bind:this={audio} onended={() => (playing = null)} preload="none"></audio>

{#snippet list(title: string, help: string, items: Found[])}
  <section class="panel">
    <h2>{title}</h2>
    <p class="muted small">{help}</p>
    {#if items.length}
      <ol class="tracks">
        {#each items as f (f.deezer_id)}
          <li class:liked={liked.has(f.deezer_id)}>
            <button class="play" class:on={playing === f.deezer_id} onclick={() => play(f)} aria-label={t(playing === f.deezer_id ? "dc.pause" : "dc.play")} title={t(playing === f.deezer_id ? "dc.pause" : "dc.play")}>
              {#if f.cover}<img src={f.cover} alt="" loading="lazy" />{/if}
              <span class="icon">{playing === f.deezer_id ? "❚❚" : "▶"}</span>
            </button>
            <span class="what">
              <span class="name">{f.title}</span>
              <span class="muted small">{f.artist}{#if f.album}{" · "}{f.album}{/if}</span>
              {#if why(f)}<span class="why small">{why(f)}</span>{/if}
            </span>
            <span class="mono muted small len">{f.duration_s ? duration(f.duration_s) : ""}</span>
            <span class="actions">
              <button class="btn" aria-pressed={liked.has(f.deezer_id)} onclick={() => verdict(f, liked.has(f.deezer_id) ? 0 : 1)}>{t("dc.like")}</button>
              <button class="btn" onclick={() => verdict(f, -1)}>{t("dc.dislike")}</button>
              <button class="link small" onclick={() => openUrl(`https://www.deezer.com/track/${f.deezer_id}`).catch(() => {})}>Deezer ↗</button>
            </span>
          </li>
        {/each}
      </ol>
    {:else}
      <p class="muted">{t("disc.none")}</p>
    {/if}
  </section>
{/snippet}

<div class="page">
  <header class="head">
    <div>
      <div class="eyebrow">{t("nav.discover")}</div>
      <h1>{artistId ? t("dc.titleArtist", { name: artistName }) : t("dc.title")}</h1>
      <p class="intro">{artistId ? t("dc.introArtist", { name: artistName }) : t("dc.intro")}</p>
    </div>
    <div class="ctrl">
      {#if running}
        <button class="btn" onclick={() => api.stopDiscover()}>{t("acc.stop")}</button>
      {:else}
        <button class="btn primary" onclick={start}>{sel?.computed_at ? t("dc.refresh") : t("dc.start")}</button>
      {/if}
    </div>
  </header>

  {#if running}
    <div class="progress">
      <span class="bar"><span style:width={`${status && status.total ? (100 * status.done) / status.total : 3}%`}></span></span>
      <span class="muted small">{status?.current ? t("dc.running", { name: status.current }) : t("dc.starting")}</span>
    </div>
  {/if}
  {#if error}<p class="err" role="alert">{t("error", { msg: error })}</p>{/if}
  {#if message}<p class="muted small">{message}</p>{/if}

  {#if sel?.computed_at}
    <div class="meta">
      <span class="muted small">{t("dc.computed", { date: date(sel.computed_at) })}</span>
      <span class="exports">
        <span class="muted small">{t("mx.export")}</span>
        {#each ["xspf", "jspf", "csv"] as const as f (f)}<button class="btn" onclick={() => exportAll(f)}>{f.toUpperCase()}</button>{/each}
      </span>
    </div>
    {@render list(t("dc.new"), t("dc.newHelp"), sel.new_artists)}
    {@render list(t("dc.known"), t("dc.knownHelp"), sel.known_artists)}
    <p class="muted small">{t("dc.note")}</p>
  {:else if sel && !running}
    <div class="panel empty">
      <p>{t("dc.empty")}</p>
    </div>
  {/if}
</div>

<style>
  .page { display: grid; gap: 18px; max-width: 1100px; }
  .head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: end; gap: 12px; }
  .intro { margin: 6px 0 0; max-width: 75ch; color: var(--fg-2); }
  p { margin: 0; }
  .progress { display: grid; gap: 6px; }
  .bar { display: block; height: 4px; border-radius: 2px; background: var(--chip); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width 0.4s; }
  .err { color: var(--danger); }
  .meta { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 10px; align-items: center; }
  .exports { display: flex; gap: 8px; align-items: center; }
  .exports .btn, .actions .btn { padding: 3px 10px; font-size: 13px; }
  .tracks { list-style: none; margin: 0; padding: 0; display: grid; }
  .tracks li { display: grid; grid-template-columns: 52px minmax(0, 1fr) auto auto; gap: 4px 14px; align-items: center; padding: 8px 0; border-bottom: 1px solid var(--line); }
  .tracks li:last-child { border-bottom: 0; }
  .tracks li.liked .name { color: var(--accent); }
  .play { position: relative; width: 52px; height: 52px; padding: 0; border: 0; border-radius: 6px; overflow: hidden; background: var(--chip); cursor: pointer; }
  .play img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .play .icon { position: absolute; inset: 0; display: grid; place-items: center; color: #fff; background: rgb(0 0 0 / 0.35); font-size: 15px; opacity: 0; transition: opacity 0.15s; }
  .play:hover .icon, .play.on .icon, .play:focus-visible .icon { opacity: 1; }
  .what { display: grid; min-width: 0; }
  .what > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .name { font-weight: 500; }
  .why { color: var(--fg-3); }
  .len { min-width: 3em; text-align: right; }
  .actions { display: flex; gap: 6px; align-items: center; }
  .actions .btn[aria-pressed="true"] { border-color: var(--accent); color: var(--accent); }
  .link { background: none; border: 0; color: var(--accent); padding: 0; cursor: pointer; }
  .empty p { color: var(--fg-2); }
  @media (max-width: 760px) {
    .tracks li { grid-template-columns: 52px minmax(0, 1fr); }
    .len { display: none; }
    .actions { grid-column: 2; }
  }
</style>
