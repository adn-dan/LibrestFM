<script lang="ts">
  // Sessions d'écoute (écoutes enchaînées à moins de 30 min d'écart) sur la période.
  import { api, entityHref, type Period, type Ranked, type SessionStats } from "./api";
  import Chart from "./Chart.svelte";
  import Cover from "./Cover.svelte";
  import { axisStyle, base, type ThemeColors } from "./theme";
  import { date, minutes, num } from "./format";
  import { locale, t } from "./settings.svelte";
  import { openDrill } from "./drill.svelte";

  let { period, subtitle }: { period: Period; subtitle?: string } = $props();
  let s = $state<SessionStats | null>(null);

  $effect(() => {
    let alive = true;
    api
      .sessions(period)
      .then((r) => alive && (s = r))
      .catch(() => {});
    return () => (alive = false);
  });

  function columns(labels: string[], values: number[], tip: (i: number) => string) {
    return (c: ThemeColors) => ({
      ...base(c),
      grid: { left: 40, right: 8, top: 10, bottom: 24 },
      tooltip: { ...base(c).tooltip, trigger: "axis" as const, formatter: (p: any) => `${tip(p[0].dataIndex)}<br><b>${t("ses.sessions", { n: p[0].value })}</b>` },
      xAxis: { type: "category" as const, data: labels, ...axisStyle(c) },
      yAxis: { type: "value" as const, minInterval: 1, ...axisStyle(c) },
      series: [{ type: "bar" as const, data: values, barMaxWidth: 22, itemStyle: { color: c.series[0], borderRadius: [3, 3, 0, 0] } }],
    });
  }
</script>

<section class="panel">
  <h2>{t("ses.title")}</h2>
  <p class="muted small">{t("ses.help")}</p>
  {#if s && s.count}
    <dl class="kpis">
      <div class="kpi"><dt>{t("ses.count")}</dt><dd>{num(s.count)}</dd></div>
      <div class="kpi"><dt>{t("ses.avg")}</dt><dd>{new Intl.NumberFormat(locale(), { maximumFractionDigits: 1 }).format(s.avg_tracks)}</dd></div>
      <div class="kpi"><dt>{t("ses.median")}</dt><dd>{minutes(s.median_minutes)}</dd></div>
      {#if s.longest}
        <div class="kpi">
          <dt>{t("ses.longest")}</dt>
          <dd>{minutes(s.longest.minutes)} <small>{t("ses.longestValue", { tracks: s.longest.tracks, date: date(s.longest.start_ts) })}</small></dd>
        </div>
      {/if}
    </dl>
    <div class="two">
      <div>
        <h3>{t("ses.lengths")}</h3>
        <Chart option={columns(s.lengths.map((l) => l[0]), s.lengths.map((l) => l[1]), (i) => s!.lengths[i][0])} height="200px" label={t("ses.lengths")} />
      </div>
      <div>
        <h3>{t("ses.hours")}</h3>
        <Chart
          option={columns(s.start_hours.map((_, h) => String(h)), s.start_hours, (i) => t("ses.hourTip", { h: String(i) }))}
          height="200px"
          label={t("ses.hours")}
          onclick={(p) => openDrill(t("ses.hourTip", { h: String(p.dataIndex) }), { ...period, hour: p.dataIndex }, { subtitle: subtitle ?? "" })}
        />
      </div>
    </div>
    <div class="two">
      {#each [["ses.openers", s.openers], ["ses.closers", s.closers]] as [title, list] (title)}
        <div>
          <h3>{t(title as "ses.openers")}</h3>
          <ol class="who">
            {#each list as Ranked[] as r (r.id)}
              <li>
                <Cover kind="artist" id={r.id} name={r.name} size={28} />
                <a href={entityHref("artist", r.id)}>{r.name}</a>
                <span class="mono muted">{t("ses.sessions", { n: r.plays })}</span>
              </li>
            {/each}
          </ol>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .two { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 16px 28px; }
  h3 { font-family: var(--font-body); font-weight: 500; font-size: 13.5px; margin: 0 0 4px; color: var(--fg-2); }
  p { margin: 0; }
  .who { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .who li { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 10px; align-items: center; }
  .who a { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
