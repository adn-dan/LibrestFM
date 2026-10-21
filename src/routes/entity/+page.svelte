<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { api, entityHref, personHref, type CreditedArtist, type EntityDetail, type Kind, type Profile, type Ranked } from "$lib/api";
  import Chart from "$lib/Chart.svelte";
  import { axisStyle, base, type ThemeColors } from "$lib/theme";
  import TimeChart from "$lib/TimeChart.svelte";
  import Cover from "$lib/Cover.svelte";
  import Credits from "$lib/Credits.svelte";
  import LyricsPanel from "$lib/LyricsPanel.svelte";
  import MergeWith from "$lib/MergeWith.svelte";
  import ArtistLinks from "$lib/ArtistLinks.svelte";
  import ArtistCredits from "$lib/ArtistCredits.svelte";
  import { openDrill } from "$lib/drill.svelte";
  import { axes, notes, date, esc, month, num, duration, partialDate, country, hours, errText } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  const kindLabel = (k: Kind) => t(`kind.${k}` as const);
  /** Libellé traduit d'un profil du radar. */
  function profileLabel(p: Profile): string {
    if (p.kind === "self") return p.name ?? "";
    if (p.kind === "library") return t("profile.library");
    return t(p.kind === "album" ? "profile.album" : "profile.artist", { name: p.name ?? "" });
  }

  let d = $state<EntityDetail | null>(null);
  /** Artistes crédités relus depuis l'ouverture (fiche ancienne complétée). */
  let freshCredit = $state<CreditedArtist[] | null>(null);
  const credit = $derived(freshCredit ?? d?.artist_credit ?? []);
  /** Incrémenté pour recharger la fiche (après une annulation de fusion). */
  let reload = $state(0);
  let error = $state("");
  let scale = $state<"raw" | "pct">("raw");
  /** Profils superposés (indices dans d.profiles, le premier est toujours affiché). */
  let shown = $state<boolean[]>([]);

  $effect(() => {
    void reload;
    const kind = page.url.searchParams.get("kind") as Kind;
    const id = Number(page.url.searchParams.get("id"));
    d = null;
    freshCredit = null;
    error = "";
    let alive = true;
    const load = () =>
      api.entity(kind, id).then((r) => {
        if (!alive) return;
        d = r;
        shown = r.profiles.map((_, i) => i < 2);
      });
    load()
      // Infos d'album / d'artiste manquantes : récupérées maintenant, puis fiche rechargée.
      .then(() => (kind !== "track" ? api.ensureInfo(kind, id) : false))
      .then((fetched) => {
        if (fetched && alive) return load();
      })
      .catch((e) => alive && (error = errText(e)));
    return () => (alive = false);
  });

  const peak = $derived(d?.per_month.reduce((a, b) => (b[1] > a[1] ? b : a), ["", 0] as [string, number]));



  function bars(list: Ranked[]) {
    return (c: ThemeColors) => ({
      ...base(c),
      grid: { left: 8, right: 52, top: 4, bottom: 4, containLabel: true },
      tooltip: { ...base(c).tooltip, formatter: (p: any) => `<b>${esc(list[p.dataIndex].name)}</b><br>${t("plays", { n: p.value })}` },
      xAxis: { type: "value" as const, show: false },
      yAxis: {
        type: "category" as const,
        inverse: true,
        data: list.map((r) => r.name),
        ...axisStyle(c),
        axisLine: { show: false },
        axisLabel: { color: c.fg2, width: 200, overflow: "truncate" as const },
      },
      series: [
        {
          type: "bar" as const,
          data: list.map((r) => r.plays),
          barMaxWidth: 16,
          itemStyle: { color: c.series[0], borderRadius: [0, 3, 3, 0] },
          label: { show: true, position: "right" as const, color: c.fg3, formatter: (p: any) => num(p.value) },
          cursor: "pointer",
        },
      ],
    });
  }

  function radar(c: ThemeColors) {
    if (!d) return {};
    const pct = scale === "pct";
    const AXES = axes();
    const series = d.profiles
      .map((p, i) => ({ p, i }))
      .filter(({ i }) => i === 0 || shown[i])
      .reverse(); // le profil principal dessiné en dernier, au-dessus
    return {
      ...base(c),
      tooltip: {
        ...base(c).tooltip,
        trigger: "item" as const,
        formatter: (x: any) =>
          `<b>${esc(x.name)}</b><br>` +
          AXES.map((a, j) => `${a.label}${t("colon")}<b>${x.value[j] == null ? "–" : pct ? Math.round(x.value[j]) : x.value[j].toFixed(2)}</b>`).join("<br>"),
      },
      radar: {
        radius: "66%",
        center: ["50%", "54%"],
        splitNumber: 4,
        indicator: AXES.map((a) => ({ name: a.label, min: 0, max: pct ? 100 : 1 })),
        axisName: { color: c.fg2, fontSize: 13 },
        splitLine: { lineStyle: { color: c.line } },
        axisLine: { lineStyle: { color: c.line } },
        splitArea: { show: false },
      },
      series: [
        {
          type: "radar" as const,
          symbol: "circle",
          data: series.map(({ p, i }) => {
            const col = c.series[Math.min(i, 3)];
            return {
              name: profileLabel(p),
              value: (pct ? p.percentiles : p.axes).map((v) => v ?? 0),
              symbolSize: i === 0 ? 8 : 6,
              lineStyle: { color: col, width: 2, type: DASHES[Math.min(i, 3)] },
              itemStyle: { color: col, borderColor: c.surface, borderWidth: 2 },
              areaStyle: { color: col, opacity: i === 0 ? 0.22 : 0.06 },
            };
          }),
        },
      ],
    };
  }

  const fmt = (v: number | null | undefined) => (v == null ? "–" : v.toFixed(2));
  const SERIES_VARS = ["--s1", "--s2", "--s3", "--s4"];
  // La couleur n'est pas le seul repère : chaque superposition a son trait.
  const DASHES: ("solid" | number[])[] = ["solid", [6, 4], [2, 3], [10, 3, 2, 3]];
</script>

{#if error}
  <p class="panel" role="alert">{t("error", { msg: error })}</p>
{:else if d}
  <div class="page">
    <header class="head">
      <div class="ident">
        <Cover kind={d.kind} id={d.id} name={d.name} size={136} />
        <div class="titles">
          <div class="eyebrow">{kindLabel(d.kind)}</div>
          <h1>{d.name}</h1>
          {#if credit.length}
            <!-- Plusieurs artistes crédités : l'ordre de MusicBrainz, le principal d'abord. -->
            <span class="by">
              {#each credit as c (c.person_id)}<a href={personHref(c.person_id, c.artist_id)}>{c.name}</a>{c.join}{/each}
            </span>
          {:else if d.artist && d.artist_id}
            <a class="by" href={entityHref("artist", d.artist_id)}>{d.artist}</a>
          {/if}
          <span class="mixbtns">
            <a class="btn mixbtn" href={`/mixes?seed=${d.kind}&id=${d.id}`}>{t(`mx.from.${d.kind}`)}</a>
            {#if d.kind === "artist"}<a class="btn mixbtn" href={`/discover?artist=${d.id}&name=${encodeURIComponent(d.name)}`}>{t("dc.fromArtist")}</a>{/if}
          </span>
        </div>
      </div>
      {#if d.merged.length}
        <p class="merged small muted">
          {t("mg.includes")}{t("colon")}
          {#each d.merged as m, i (m.id)}
            {i > 0 ? " · " : ""}{m.label.split(" → ")[0]}
            <button class="link" onclick={async () => { await api.undoMerge(m.id); reload++; }}>({t("cl.undo").toLowerCase()})</button>
          {/each}
        </p>
      {/if}
      <dl class="kpis">
        <div class="kpi"><dt>{t("kpi.plays")}</dt><dd>{num(d.total)}</dd></div>
        {#if d.listened_s}<div class="kpi"><dt>{t("kpi.listened")}</dt><dd>≈ {hours(d.listened_s)}</dd></div>{/if}
        <div class="kpi"><dt>{t("kpi.first")}</dt><dd>{date(d.first_ts)}</dd></div>
        <div class="kpi"><dt>{t("kpi.last")}</dt><dd>{date(d.last_ts)}</dd></div>
        {#if peak && peak[1] > 0}<div class="kpi"><dt>{t("kpi.peak")}</dt><dd>{month(peak[0])} <small>{num(peak[1])}</small></dd></div>{/if}
        {#if d.duration_ms}
          <div class="kpi">
            <dt>{t("kpi.duration")}</dt>
            <dd>{duration(d.duration_ms / 1000)}{#if d.kind === "album" && d.album_info?.nb_tracks} <small>{t("tracksCount", { n: d.album_info.nb_tracks })}</small>{/if}</dd>
          </div>
        {/if}
        {#if d.kind !== "artist" && d.album_info?.release_date}
          {@const ai = d.album_info}
          <div class="kpi">
            <dt>{t("kpi.release")}</dt>
            <dd>
              {partialDate(ai.release_date)}
              {#if ai.edition_date && ai.edition_date.slice(0, 4) !== ai.release_date?.slice(0, 4)}<small>{t("kpi.edition", { date: ai.edition_date.slice(0, 4) })}</small>{/if}
            </dd>
          </div>
        {/if}
        {#if d.kind === "album" && d.album_info?.label}<div class="kpi"><dt>{t("kpi.label")}</dt><dd class="text">{d.album_info.label}</dd></div>{/if}
        {#if d.kind === "artist" && d.artist_info?.status === "ok"}
          {@const ar = d.artist_info}
          {#if ar.country || ar.area}
            <div class="kpi">
              <dt>{t("kpi.origin")}</dt>
              <dd class="text">{[ar.area, country(ar.country)].filter((x, i, a) => x && x !== "–" && a.indexOf(x) === i).join(", ")}{#if ar.kind}<small>{t(`akind.${ar.kind}` as "akind.Other")}</small>{/if}</dd>
            </div>
          {/if}
          {#if ar.begin}<div class="kpi"><dt>{t("kpi.lifespan")}</dt><dd>{partialDate(ar.begin)}{ar.end ? ` – ${partialDate(ar.end)}` : ""}</dd></div>{/if}
        {/if}
        {#if d.features?.status === "ok"}
          {@const f = d.features}
          {#if f.tempo != null}<div class="kpi"><dt>{t("kpi.tempo")}</dt><dd>{Math.round(f.tempo)} <small>BPM</small></dd></div>{/if}
          {#if f.key != null}<div class="kpi"><dt>{t("kpi.key")}</dt><dd>{notes()[f.key]} {f.mode === 1 ? t("mode.major") : f.mode === 0 ? t("mode.minor") : ""}</dd></div>{/if}
          {#if f.loudness != null}<div class="kpi"><dt>{t("kpi.loudness")}</dt><dd>{f.loudness.toFixed(1)} <small>dB</small></dd></div>{/if}
        {/if}
      </dl>
    </header>

    <section class="panel">
      <h2>{t("ov.timeline")}</h2>
      <TimeChart
        perMonth={d.per_month}
        perDay={d.per_day}
        height="240px"
        onpick={(from, to, label) => d && openDrill(`${d.name} · ${label}`, { kind: d.kind, id: d.id, from, to }, { timeline: false })}
      />
    </section>

    {#if d.kind !== "track"}{@render tops(d)}{/if}

    {#if d.profiles.length}
      <div class="two">
        <section class="panel">
          <div class="panel-head">
            <h2>{t("feat.title")}</h2>
            <div class="seg" role="group" aria-label={t("feat.scale")}>
              <button aria-pressed={scale === "raw"} onclick={() => (scale = "raw")}>{t("feat.values")}</button>
              <button aria-pressed={scale === "pct"} onclick={() => (scale = "pct")}>{t("feat.percentiles")}</button>
            </div>
          </div>
          <div class="legend">
            {#each d.profiles as p, i (p.kind)}
              <label class="toggle" style:--c={`var(${SERIES_VARS[Math.min(i, 3)]})`} class:fixed={i === 0}>
                <input type="checkbox" bind:checked={shown[i]} disabled={i === 0} />
                <span class="sw" class:off={i > 0 && !shown[i]}></span>{profileLabel(p)}
              </label>
            {/each}
          </div>
          <Chart option={radar} height="380px" label={t("feat.aria")} />
          <p class="muted small">
            {scale === "pct" ? t("feat.hint.pct") : t("feat.hint.raw")}
            {#if d.features?.status === "ok"}
              {t("feat.source", { title: d.features.matched_title ?? "" })}
            {/if}
          </p>
        </section>

        <section class="panel">
          <h2>{t("feat.details")}</h2>
          <div class="tablewrap">
            <table>
              <thead>
                <tr>
                  <th>{t("feat.axis")}</th>
                  {#each d.profiles as p, i (p.kind)}
                    {#if i === 0 || shown[i]}<th class="num"><span class="k" style:--c={`var(${SERIES_VARS[Math.min(i, 3)]})`}></span>{i === 0 ? t("feat.value") : profileLabel(p)}</th>{/if}
                  {/each}
                  <th class="num">{t("feat.rank")}</th>
                </tr>
              </thead>
              <tbody>
                {#each axes() as a, j (a.label)}
                  {@const r = d.profiles[0].percentiles[j]}
                  <tr>
                    <td>{a.label}<div class="muted small">{a.help}</div></td>
                    {#each d.profiles as p, i (p.kind)}
                      {#if i === 0 || shown[i]}<td class="num">{fmt(p.axes[j])}</td>{/if}
                    {/each}
                    <td class="num">
                      {#if r != null}<span class="rank"><span class="bar"><i style:width={`${r}%`}></i></span>{Math.round(r)}</span>{:else}–{/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </section>
      </div>
    {:else if d.kind === "track"}
      <section class="panel">
        <h2>{t("feat.title")}</h2>
        <p class="muted">
          {#if d.features?.status === "not_found"}
            {t("feat.notFound")}
          {:else}
            {t("feat.notYet")} <a href="/enrich">{t("feat.start")}</a>.
          {/if}
        </p>
      </section>
    {/if}

    {#if d.kind !== "artist"}<Credits kind={d.kind} id={d.id} oncredit={(c) => (freshCredit = c)} />{:else}<ArtistCredits id={d.id} name={d.name} /><ArtistLinks id={d.id} name={d.name} />{/if}

    {#if d.kind === "track" && d.artist}
      <LyricsPanel artist={d.artist} title={d.name} album={d.top_albums[0]?.name ?? null} durationMs={d.duration_ms} />
    {/if}

    <div class="two">
      <section class="panel">
        <h2>{t("nb.before")}</h2>
        {@render neighbourList(d.before)}
      </section>
      <section class="panel">
        <h2>{t("nb.after")}</h2>
        {@render neighbourList(d.after)}
      </section>
    </div>
    <p class="muted small">
      {t("nb.note", { level: t(`kinds.${d.kind}.lower` as const), name: d.name })}
    </p>

    {#if d.kind === "track"}{@render tops(d)}{/if}

    {#if d.versions.length}
      <section class="panel">
        <h2>{t("ver.title")}</h2>
        <ol class="nb">
          {#each d.versions as v (v.id)}
            <li>
              <Cover kind="track" id={v.id} name={v.name} size={36} />
              <span class="nbname"><a href={entityHref("track", v.id)}>{v.name}</a></span>
              <span class="mono muted">{num(v.plays)}×</span>
            </li>
          {/each}
        </ol>
        <p class="muted small">{t("ver.total", { n: t("plays", { n: d.total + d.versions.reduce((s, v) => s + v.plays, 0) }) })}</p>
      </section>
    {/if}

    <section class="panel">
      <MergeWith kind={d.kind} id={d.id} name={d.name} />
    </section>
  </div>
{/if}

{#snippet tops(d: EntityDetail)}
  <div class="two">
    {#if d.top_tracks.length}
      {@const list = d.top_tracks.slice(0, 20)}
      <section class="panel">
        <h2>{t("top.tracks")}</h2>
        <Chart option={bars(list)} height={`${list.length * 26 + 8}px`} label={t("top.tracks")} onclick={(p) => goto(entityHref("track", list[p.dataIndex].id))} />
      </section>
    {/if}
    {#if d.top_albums.length}
      {@const list = d.top_albums.slice(0, 20)}
      <section class="panel">
        <h2>{d.kind === "track" ? t("top.onAlbums") : t("top.albums")}</h2>
        <Chart option={bars(list)} height={`${list.length * 26 + 8}px`} label={t("top.albums")} onclick={(p) => goto(entityHref("album", list[p.dataIndex].id))} />
      </section>
    {/if}
  </div>
{/snippet}

{#snippet neighbourList(list: Ranked[])}
  {#if list.length}
    <ol class="nb">
      {#each list as r (r.id)}
        <li>
          <Cover kind={r.kind} id={r.id} name={r.name} size={36} />
          <span class="nbname">
            <a href={entityHref(r.kind, r.id)}>{r.name}</a>
            {#if r.artist}<span class="muted small">{r.artist}</span>{/if}
          </span>
          <span class="mono muted">{num(r.plays)}×</span>
        </li>
      {/each}
    </ol>
  {:else}
    <p class="muted">{t("nb.none")}</p>
  {/if}
{/snippet}

<style>
  .page { display: grid; gap: 20px; max-width: 1400px; }
  .head { display: grid; gap: 6px; }
  .by { font-size: 18px; color: var(--fg-2); }
  .by a { color: inherit; }
  .kpis { padding-top: 10px; }
  .kpi dd.text { font-family: var(--font-body); font-size: 19px; }
  .kpi dd small { margin-left: 6px; }
  .two { display: grid; grid-template-columns: repeat(auto-fit, minmax(380px, 1fr)); gap: 20px; }
  .legend { display: flex; flex-wrap: wrap; gap: 8px; }
  .toggle { display: inline-flex; align-items: center; gap: 7px; font-size: 13px; background: var(--chip); border-radius: 999px; padding: 4px 11px 4px 9px; cursor: pointer; }
  .toggle.fixed { cursor: default; }
  .toggle input { position: absolute; opacity: 0; pointer-events: none; }
  .toggle:has(input:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .sw { width: 12px; height: 12px; border-radius: 3px; border: 2px solid var(--c); background: var(--c); }
  .sw.off { background: transparent; }
  .k { display: inline-block; width: 9px; height: 9px; border-radius: 2px; background: var(--c); margin-right: 5px; }
  .rank { display: inline-grid; grid-template-columns: 54px 3ch; gap: 6px; align-items: center; }
  .bar { height: 6px; border-radius: 3px; background: var(--chip); position: relative; overflow: hidden; }
  .bar i { position: absolute; inset: 0 auto 0 0; background: var(--s1); border-radius: 3px; }
  .merged { margin: 0; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; font: inherit; cursor: pointer; }
  .ident { display: flex; gap: 20px; align-items: flex-end; min-width: 0; }
  .titles { display: grid; gap: 4px; min-width: 0; }
  .mixbtns { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 6px; }
  .mixbtn { padding: 4px 12px; font-size: 13px; text-decoration: none; }
  .nb { margin: 0; padding: 0; list-style: none; display: grid; gap: 8px; }
  .nb li { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 10px; align-items: center; }
  .nbname { display: grid; min-width: 0; }
  .nbname > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
