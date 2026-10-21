<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, entityHref, type Overview, type Period, type Ranked } from "$lib/api";
  import Chart from "$lib/Chart.svelte";
  import { axisStyle, base, hbars, type ThemeColors } from "$lib/theme";
  import TimeChart from "$lib/TimeChart.svelte";
  import ReleaseYears from "$lib/ReleaseYears.svelte";
  import Sessions from "$lib/Sessions.svelte";
  import Discoveries from "$lib/Discoveries.svelte";
  import WorldMap from "$lib/WorldMap.svelte";
  import { openDrill } from "$lib/drill.svelte";
  import { lyricsLanguage } from "$lib/roles";
  import { country, date, esc, hours, num, weekdays, errText } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  type Choice = { id: string; label: string; period: Period };

  let data = $state<Overview | null>(null);
  /** Historique complet pour la courbe : la période ne fait que cadrer le zoom. */
  let history = $state<Pick<Overview, "per_month" | "per_day"> | null>(null);
  let error = $state("");
  let years = $state<number[]>([]);
  let choice = $state("all");
  let loaded = $state(false);
  /** Classements au temps d'écoute plutôt qu'au nombre d'écoutes. */
  let byTime = $state(false);

  const choices: Choice[] = $derived.by(() => {
    const now = Math.floor(Date.now() / 1000);
    const list: Choice[] = [
      { id: "all", label: t("period.all"), period: { from: null, to: null } },
      { id: "12m", label: t("period.12m"), period: { from: now - 365 * 86400, to: null } },
    ];
    for (const y of years) {
      list.push({
        id: String(y),
        label: String(y),
        period: { from: new Date(y, 0, 1).getTime() / 1000, to: new Date(y + 1, 0, 1).getTime() / 1000 - 1 },
      });
    }
    return list;
  });

  const period = $derived(choices.find((c) => c.id === choice)?.period ?? { from: null, to: null });
  /** Rappel de la période dans le panneau de détail (vide pour « Tout »). */
  const periodNote = $derived(choice === "all" ? "" : t("dr.period", { label: choices.find((c) => c.id === choice)?.label ?? "" }));
  const drillCountry = (code: string) => openDrill(country(code), { ...period, country: code }, { subtitle: periodNote });
  const drillLanguage = (code: string) => openDrill(lyricsLanguage(code) ?? code, { ...period, language: code }, { subtitle: periodNote });

  function originsChart(c: ThemeColors) {
    const rows = (data?.countries ?? []).slice(0, 15);
    return hbars(c, rows.map((r) => country(r[0])), rows.map((r) => r[1]),
      (i) => `<b>${esc(country(rows[i][0]))}</b><br>${t("plays", { n: rows[i][1] })}<br>${t("orig.artists", { n: rows[i][2] })}`, num);
  }

  function languagesChart(c: ThemeColors) {
    const rows = (data?.languages ?? []).slice(0, 15);
    return hbars(c, rows.map((r) => lyricsLanguage(r[0]) ?? r[0]), rows.map((r) => r[1]),
      (i) => `<b>${esc(lyricsLanguage(rows[i][0]) ?? rows[i][0])}</b><br>${t("plays", { n: rows[i][1] })}`, num);
  }

  const zoomRange = $derived.by((): [number, number] | null => {
    const p = choices.find((c) => c.id === choice)?.period;
    if (!p || (p.from == null && p.to == null)) return null;
    return [(p.from ?? 0) * 1000, (p.to ?? Date.now() / 1000) * 1000];
  });

  async function load(id: string) {
    choice = id;
    const c = choices.find((c) => c.id === id) ?? choices[0];
    try {
      data = await api.overview(c.period, byTime);
      error = "";
    } catch (e) {
      error = errText(e);
    }
  }

  $effect(() => {
    // Premier chargement : toute la période, pour connaître les années disponibles.
    api
      .overview({ from: null, to: null })
      .then((o) => {
        data = o;
        history = { per_month: o.per_month, per_day: o.per_day };
        if (o.first_ts && o.last_ts) {
          const a = new Date(o.first_ts * 1000).getFullYear();
          const b = new Date(o.last_ts * 1000).getFullYear();
          years = Array.from({ length: b - a + 1 }, (_, i) => b - i);
        }
      })
      .catch((e) => (error = errText(e)))
      .finally(() => (loaded = true));
  });



  function topChart(list: Ranked[]) {
    return (c: ThemeColors) => ({
      ...base(c),
      grid: { left: 8, right: 56, top: 4, bottom: 4, containLabel: true },
      tooltip: {
        ...base(c).tooltip,
        formatter: (p: any) => {
          const r = list[p.dataIndex];
          const time = r.seconds != null ? `<br>${hours(r.seconds)}` : "";
          return `<b>${esc(r.name)}</b>${r.artist ? `<br>${esc(r.artist)}` : ""}<br>${t("plays", { n: r.plays })}${time}`;
        },
      },
      xAxis: { type: "value" as const, show: false },
      yAxis: {
        type: "category" as const,
        inverse: true,
        data: list.map((r) => r.name),
        ...axisStyle(c),
        axisLine: { show: false },
        axisLabel: { color: c.fg2, width: 170, overflow: "truncate" as const },
      },
      series: [
        {
          type: "bar" as const,
          data: list.map((r) => r.seconds ?? r.plays),
          barMaxWidth: 16,
          itemStyle: { color: c.series[0], borderRadius: [0, 3, 3, 0] },
          label: { show: true, position: "right" as const, color: c.fg3, fontFamily: "IBM Plex Mono", formatter: (p: any) => (list[p.dataIndex].seconds != null ? hours(p.value) : num(p.value)) },
          cursor: "pointer",
        },
      ],
    });
  }

  function heatmap(c: ThemeColors) {
    const DAYS = weekdays();
    const h = data?.heatmap ?? [];
    const max = Math.max(1, ...h.map((x) => x[2]));
    return {
      ...base(c),
      grid: { left: 40, right: 16, top: 8, bottom: 56 },
      tooltip: { ...base(c).tooltip, formatter: (p: any) => `${t("ov.heat.tip", { day: DAYS[p.value[1]], h: String(p.value[0]) })}<br><b>${t("plays", { n: p.value[2] })}</b>` },
      xAxis: { type: "category" as const, data: Array.from({ length: 24 }, (_, i) => i), ...axisStyle(c), splitArea: { show: false } },
      yAxis: { type: "category" as const, data: DAYS, inverse: true, ...axisStyle(c) },
      visualMap: { min: 0, max, orient: "horizontal" as const, left: "center", bottom: 0, itemHeight: 140, inRange: { color: c.seq }, textStyle: { color: c.fg3 }, calculable: false },
      series: [{ type: "heatmap" as const, data: h.map(([d, hr, n]) => [hr, d, n]), itemStyle: { borderColor: c.surface, borderWidth: 2, borderRadius: 3 } }],
    };
  }

  const tops = $derived(
    data
      ? ([
          [t("ov.top.artist"), data.top_artists],
          [t("ov.top.album"), data.top_albums],
          [t("ov.top.track"), data.top_tracks],
        ] as [string, Ranked[]][])
      : [],
  );
</script>

{#if error}
  <p class="panel" role="alert">{t("error", { msg: error })}</p>
{:else if loaded && data && data.total === 0 && choice === "all"}
  <div class="panel empty">
    <h1>{t("ov.empty.title")}</h1>
    <p>{t("ov.empty.text")}</p>
    <a class="btn primary" href="/import">{t("ov.empty.button")}</a>
  </div>
{:else if data}
  <div class="page">
    <header class="head">
      <div>
        <div class="eyebrow">{t("nav.overview")}</div>
        <h1>{choice === "all" ? t("ov.allTitle") : choices.find((c) => c.id === choice)?.label}</h1>
      </div>
      <label class="period">
        {t("period.label")}
        <select value={choice} onchange={(e) => load(e.currentTarget.value)}>
          {#each choices as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
        </select>
      </label>
    </header>

    <dl class="kpis">
      <div class="kpi"><dt>{t("kpi.plays")}</dt><dd>{num(data.total)}</dd></div>
      <div class="kpi" title={t("kpi.listened.tip", { min: hours(data.listened_min_s), p: Math.round(data.listened_known * 100) })}>
        <dt>{t("kpi.listened")}</dt><dd>≈ {hours(data.listened_s)}</dd>
      </div>
      <div class="kpi"><dt>{t("kinds.artist")}</dt><dd>{num(data.artists)}</dd></div>
      <div class="kpi"><dt>{t("kinds.album")}</dt><dd>{num(data.albums)}</dd></div>
      <div class="kpi"><dt>{t("kinds.track")}</dt><dd>{num(data.tracks)}</dd></div>
      <div class="kpi"><dt>{t("kpi.from")}</dt><dd>{date(data.first_ts)}</dd></div>
      <div class="kpi"><dt>{t("kpi.to")}</dt><dd>{date(data.last_ts)}</dd></div>
    </dl>

    <section class="panel">
      <h2>{t("ov.timeline")}</h2>
      {#if history}<TimeChart
          perMonth={history.per_month}
          perDay={history.per_day}
          range={zoomRange}
          onpick={(from, to, label) => openDrill(label, { from, to }, { timeline: false })}
        />{/if}
    </section>

    <div class="by">
      <span class="muted small">{t("ov.by")}</span>
      <div class="seg" role="group" aria-label={t("ov.by")}>
        <button aria-pressed={!byTime} onclick={() => { byTime = false; load(choice); }}>{t("ov.by.plays")}</button>
        <button aria-pressed={byTime} onclick={() => { byTime = true; load(choice); }}>{t("ov.by.time")}</button>
      </div>
    </div>

    <div class="tops">
      {#each tops as [title, list] (title)}
        <section class="panel">
          <h2>{title}</h2>
          {#if list.length}
            <Chart
              option={topChart(list)}
              height={`${Math.max(120, list.length * 26 + 8)}px`}
              label={title}
              onclick={(p) => goto(entityHref(list[p.dataIndex].kind, list[p.dataIndex].id))}
            />
          {:else}
            <p class="muted">{t("ov.noData")}</p>
          {/if}
        </section>
      {/each}
    </div>

    <ReleaseYears years={data.release_years} covered={data.release_covered} {period} subtitle={periodNote} />

    <div class="tops">
      <section class="panel">
        <h2>{t("orig.title")}</h2>
        {#if data.countries.length}
          <WorldMap rows={data.countries} onpick={drillCountry} />
          <Chart
            option={originsChart}
            height={`${Math.min(15, data.countries.length) * 26 + 8}px`}
            label={t("orig.title")}
            onclick={(p) => drillCountry(data!.countries[p.dataIndex][0])}
          />
          <p class="muted small">{t("orig.covered", { p: Math.round(data.countries_covered * 100) })}</p>
        {:else}
          <p class="muted">{t("orig.none")}</p>
        {/if}
      </section>
      <section class="panel">
        <h2>{t("lang.title")}</h2>
        {#if data.languages.length}
          <Chart
            option={languagesChart}
            height={`${Math.min(15, data.languages.length) * 26 + 8}px`}
            label={t("lang.title")}
            onclick={(p) => drillLanguage(data!.languages[p.dataIndex][0])}
          />
          <p class="muted small">{t("lang.covered", { p: Math.round(data.languages_covered * 100) })}</p>
        {:else}
          <p class="muted">{t("lang.none")}</p>
        {/if}
      </section>
    </div>

    <Discoveries {period} />

    <Sessions {period} subtitle={periodNote} />

    <section class="panel">
      <h2>{t("ov.when")}</h2>
      <p class="muted small">{t("ov.when.text")} {t("dr.clickBar")}</p>
      <Chart
        option={heatmap}
        height="300px"
        label={t("ov.when.aria")}
        onclick={(p: any) =>
          openDrill(t("ov.heat.tip", { day: weekdays()[p.value[1]], h: String(p.value[0]) }), { ...period, dow: p.value[1], hour: p.value[0] }, { subtitle: periodNote })}
      />
    </section>
  </div>
{/if}

<style>
  .page { display: grid; gap: 20px; max-width: 1400px; }
  .head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: end; gap: 12px; }
  .period { display: flex; gap: 8px; align-items: center; color: var(--fg-2); font-size: 13px; }
  select { font: inherit; color: var(--fg); background: var(--surface); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; }
  .by { display: flex; gap: 10px; align-items: center; margin-bottom: -8px; }
  .tops { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 20px; }
</style>
