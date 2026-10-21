<script lang="ts">
  // Android : titre en cours dans une autre appli, avec ses paroles synchronisées.
  // Titre et position relus toutes les 2 s ; entre deux, la position avance à l'horloge.
  // Le titre est rapproché de la bibliothèque (corrections appliquées : titre joué sous le
  // nom d'un invité rendu à l'artiste principal), dont la fiche est accessible.
  import { api, entityHref, type Lyrics, type NowPlaying } from "$lib/api";
  import LyricsView from "$lib/LyricsView.svelte";
  import { duration } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  let now = $state<NowPlaying | null>(null);
  let base = $state({ pos: 0, at: 0 });
  let tick = $state(0);
  let lyrics = $state<Lyrics | null>(null);
  let lyricsFailed = $state(false);
  let lyricsKey = "";
  let known = $state<{ id: number; title: string; artist: string } | null>(null);

  const track = $derived(now?.track ?? null);
  const position = $derived.by(() => {
    void tick;
    if (!track) return null;
    return track.playing ? base.pos + (performance.now() - base.at) : base.pos;
  });

  async function poll() {
    const n = await api.nowPlaying().catch(() => null);
    if (!n) return;
    now = n;
    base = { pos: n.track?.positionMs ?? 0, at: performance.now() };
    const tr = n.track;
    const key = tr ? `${tr.artist}\u0001${tr.title}` : "";
    if (key === lyricsKey) return;
    lyricsKey = key;
    lyrics = null;
    lyricsFailed = false;
    known = null;
    if (!tr || !tr.artist || !tr.title) return;
    const k = await api.findTrack(tr.artist, tr.title, tr.albumArtist).catch(() => null);
    if (key !== lyricsKey) return;
    known = k;
    // Artiste corrigé d'abord, puis artiste de l'album, puis le nom annoncé par le lecteur.
    const names = [k?.artist, tr.albumArtist, tr.artist].filter((a): a is string => !!a);
    api
      .lyrics(names[0], k?.title ?? tr.title, tr.album, tr.durationMs > 0 ? tr.durationMs : null, names.slice(1))
      .then((l) => key === lyricsKey && (lyrics = l))
      .catch(() => key === lyricsKey && (lyricsFailed = true));
  }

  /** Commande du lecteur, puis titre relu (changement de titre, lecture ou pause). */
  async function control(action: "play" | "pause" | "next" | "previous") {
    await api.mediaControl(action).catch(() => {});
    setTimeout(poll, 400);
  }

  // Paroles synchronisées qui défilent pendant la lecture : l'écran reste allumé.
  const awake = $derived(!!track?.playing && !!lyrics?.lines.length);
  $effect(() => {
    if (!awake) return;
    api.keepAwake(true).catch(() => {});
    return () => void api.keepAwake(false).catch(() => {});
  });

  $effect(() => {
    poll();
    const slow = setInterval(() => document.visibilityState === "visible" && poll(), 2000);
    const fast = setInterval(() => tick++, 250);
    return () => {
      clearInterval(slow);
      clearInterval(fast);
    };
  });
</script>

<div class="page">
  {#if now && !now.access}
    <p class="muted">{t("now.noAccess")}</p>
  {:else if track}
    <!-- Reste en haut quand les paroles défilent. -->
    <header>
      <div class="eyebrow">{track.player}{#if !track.playing} · {t("now.paused")}{/if}</div>
      <h1>{#if known}<a href={entityHref("track", known.id)}>{known.title}</a>{:else}{track.title}{/if}</h1>
      <div class="by">{known?.artist ?? track.artist}{#if track.album}&nbsp;<span class="muted">· {track.album}</span>{/if}</div>
      {#if track.durationMs > 0 && position != null}
        <div class="bar"><span style:width={`${Math.min(100, (position / track.durationMs) * 100)}%`}></span></div>
        <div class="times mono small muted"><span>{duration(position / 1000)}</span><span>{duration(track.durationMs / 1000)}</span></div>
      {/if}
      <div class="controls">
        <button aria-label={t("now.previous")} onclick={() => control("previous")}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6h2v12H6zm3.5 6 8.5 6V6z" /></svg>
        </button>
        <button class="main" aria-label={track.playing ? t("now.pause") : t("now.play")} onclick={() => control(track.playing ? "pause" : "play")}>
          {#if track.playing}
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 5h4v14H6zm8 0h4v14h-4z" /></svg>
          {:else}
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z" /></svg>
          {/if}
        </button>
        <button aria-label={t("now.next")} onclick={() => control("next")}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 6h2v12h-2zM6 18l8.5-6L6 6z" /></svg>
        </button>
      </div>
    </header>
    <section class="panel">
      {#if lyrics}
        <LyricsView {lyrics} positionMs={lyrics.lines.length ? position : null} />
      {:else if lyricsFailed}
        <p class="muted">{t("lyr.error")}</p>
      {:else}
        <p class="muted">{t("lyr.loading")}</p>
      {/if}
    </section>
  {:else if now}
    <p class="muted">{t("now.none")}</p>
  {/if}
</div>

<style>
  .page { display: grid; gap: 16px; max-width: 720px; }
  header {
    position: sticky;
    top: -16px;
    z-index: 5;
    display: grid;
    gap: 4px;
    margin: -16px -14px 0;
    padding: 16px 14px 10px;
    background: var(--bg);
    border-bottom: 1px solid var(--line);
  }
  header h1 { font-size: 24px; }
  .controls { display: flex; justify-content: center; gap: 28px; margin-top: 4px; }
  .controls button { display: grid; place-items: center; width: 44px; height: 44px; border: 0; border-radius: 50%; background: none; color: var(--fg); }
  .controls button:active { background: var(--chip); }
  .controls button.main { width: 52px; height: 52px; background: var(--accent); color: #fff; }
  .controls svg { width: 24px; height: 24px; fill: currentColor; }
  .controls .main svg { width: 28px; height: 28px; }
  p { margin: 0; }
  .by { color: var(--fg-2); font-size: 16px; }
  h1 a { color: inherit; }
  .bar { height: 4px; border-radius: 2px; background: var(--chip); overflow: hidden; margin-top: 10px; }
  .bar span { display: block; height: 100%; background: var(--accent); }
  .times { display: flex; justify-content: space-between; }
</style>
