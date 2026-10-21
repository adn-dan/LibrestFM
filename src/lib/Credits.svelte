<script lang="ts">
  // Crédits (MusicBrainz) d'un titre, ou cumulés sur les titres d'un album.
  // Les crédits manquants sont récupérés à l'affichage.
  import { api, personHref, type CreditedArtist, type TrackCredit } from "./api";
  import { roleLabel, lyricsLanguage } from "./roles";
  import { t } from "./settings.svelte";

  // oncredit : artistes crédités dans l'ordre, une fois la fiche du titre lue.
  let { kind, id, oncredit }: { kind: "track" | "album"; id: number; oncredit?: (c: CreditedArtist[]) => void } = $props();

  type Row = TrackCredit & { tracks?: number };
  let rows = $state<Row[] | null>(null);
  let status = $state<"loading" | "ok" | "not_found" | "error">("loading");
  let language = $state<string | null>(null);
  /** Album : titres dont les crédits sont connus / total. */
  let progress = $state<{ done: number; total: number } | null>(null);

  $effect(() => {
    let alive = true;
    rows = null;
    status = "loading";
    language = null;
    progress = null;
    (kind === "track" ? loadTrack(() => alive) : loadAlbum(() => alive)).catch(() => alive && (status = "error"));
    return () => (alive = false);
  });

  async function loadTrack(alive: () => boolean) {
    const d = await api.trackCredits(id);
    if (!alive()) return;
    rows = d.credits;
    oncredit?.(d.artist_credit);
    language = lyricsLanguage(d.work_language);
    status = d.status === "none" ? "error" : d.status;
  }

  async function loadAlbum(alive: () => boolean) {
    let d = await api.albumCredits(id);
    const show = () => {
      rows = d.credits;
      progress = { done: d.tracks_done, total: d.tracks_total };
      status = d.credits.length ? "ok" : d.pending.length ? "loading" : "not_found";
    };
    if (!alive()) return;
    show();
    // Titres pas encore cherchés : un par un (MusicBrainz : 1 requête/s),
    // l'affichage se complète au fur et à mesure.
    for (const trackId of d.pending) {
      await api.trackCredits(trackId).catch(() => null);
      if (!alive()) return;
      d = await api.albumCredits(id);
      if (!alive()) return;
      show();
    }
  }

  type Group = "writing" | "performance" | "production" | "other";
  const GROUP_OF: Record<string, Group> = {
    writer: "writing", composer: "writing", lyricist: "writing", librettist: "writing", translator: "writing",
    credited: "performance", vocal: "performance", instrument: "performance", performer: "performance",
    conductor: "performance", "performing orchestra": "performance", "chorus master": "performance", concertmaster: "performance",
    producer: "production", engineer: "production", mix: "production", mastering: "production", recording: "production",
    programming: "production", arranger: "production", "instrument arranger": "production", "vocal arranger": "production",
    orchestrator: "production", remixer: "production", editor: "production", audio: "production", sound: "production", balance: "production",
  };
  const ORDER: Group[] = ["writing", "performance", "production", "other"];

  /** Par groupe, une ligne par personne avec tous ses rôles (et, pour un album, sur combien de titres). */
  const groups = $derived.by(() => {
    const out = new Map<Group, Map<number, { c: Row; roles: string[]; tracks: number }>>();
    for (const c of rows ?? []) {
      const g = GROUP_OF[c.role] ?? "other";
      const people = out.get(g) ?? new Map();
      const entry = people.get(c.person_id) ?? { c, roles: [], tracks: 0 };
      const label = roleLabel(c.role, c.detail);
      if (!entry.roles.includes(label)) entry.roles.push(label);
      entry.tracks = Math.max(entry.tracks, c.tracks ?? 1);
      people.set(c.person_id, entry);
      out.set(g, people);
    }
    // Album : les plus présents d'abord.
    return ORDER.filter((g) => out.has(g)).map((g) => ({ g, people: [...out.get(g)!.values()].sort((a, b) => b.tracks - a.tracks) }));
  });
</script>

<section class="panel">
  <div class="panel-head">
    <h2>{t("cred.title")}</h2>
    {#if language}<span class="muted small">{t("kpi.lyricsLang")}{t("colon")}{language}</span>{/if}
    {#if progress && progress.done < progress.total}
      <span class="muted small loading">{t("cred.album.progress", { done: progress.done, total: progress.total })}</span>
    {/if}
  </div>

  {#if status === "error"}
    <p class="muted">{t("cred.error")}</p>
  {:else if status === "loading" && !rows?.length}
    <p class="muted loading">{t("cred.loading")}</p>
  {:else if status === "not_found" || !rows?.length}
    <p class="muted">{t(kind === "album" ? "cred.album.notFound" : "cred.notFound")}</p>
  {:else}
    <div class="groups">
      {#each groups as { g, people } (g)}
        <div class="group">
          <h3 class="eyebrow">{t(`cred.group.${g}`)}</h3>
          <ul>
            {#each people as { c, roles, tracks } (c.person_id)}
              <li>
                <a href={personHref(c.person_id, c.artist_id)}>{c.name}</a>
                <span class="roles">{roles.join(", ")}</span>
                {#if kind === "album" && progress && progress.total > 1}
                  <span class="muted small">{t("cred.onTracks", { n: tracks })}</span>
                {:else if kind === "track" && c.tracks_in_library > 1}
                  <span class="link small">{t("cred.others", { n: c.tracks_in_library - 1 })}</span>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>
    <p class="muted small">{t("cred.source")}</p>
  {/if}
</section>

<style>
  .groups { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 16px 28px; }
  .group { display: grid; gap: 6px; align-content: start; min-width: 0; }
  h3 { font-family: var(--font-body); font-weight: 500; margin: 0; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  li { display: flex; flex-wrap: wrap; gap: 2px 8px; align-items: baseline; }
  .roles { color: var(--fg-2); font-size: 13px; }
  .link { color: var(--s3); }
  p { margin: 0; }
  .loading { animation: blink 1.4s ease-in-out infinite; }
  @keyframes blink { 50% { opacity: 0.5; } }
</style>
