<script lang="ts">
  // Découvertes (artistes adoptés dès les premières semaines) et abandons sur la période.
  import { api, entityHref, type Discoveries, type Period, type Turn } from "./api";
  import Chart from "./Chart.svelte";
  import Cover from "./Cover.svelte";
  import { axisStyle, base, type ThemeColors } from "./theme";
  import { date, month, num } from "./format";
  import { t } from "./settings.svelte";

  let { period }: { period: Period } = $props();
  let d = $state<Discoveries | null>(null);

  $effect(() => {
    let alive = true;
    api
      .discoveries(period)
      .then((r) => alive && (d = r))
      .catch(() => {});
    return () => (alive = false);
  });

  function perMonth(c: ThemeColors) {
    const rows = d?.per_month ?? [];
    return {
      ...base(c),
      grid: { left: 36, right: 8, top: 10, bottom: 24 },
      tooltip: { ...base(c).tooltip, trigger: "axis" as const, formatter: (p: any) => `${month(rows[p[0].dataIndex][0])}<br><b>${t("disc.newArtists", { n: p[0].value })}</b>` },
      xAxis: { type: "category" as const, data: rows.map((r) => r[0]), ...axisStyle(c), axisLabel: { ...axisStyle(c).axisLabel, formatter: (v: string) => (v.endsWith("-01") ? v.slice(0, 4) : "") } },
      yAxis: { type: "value" as const, minInterval: 1, ...axisStyle(c) },
      series: [{ type: "bar" as const, data: rows.map((r) => r[1]), barMaxWidth: 10, itemStyle: { color: c.series[0], borderRadius: [2, 2, 0, 0] } }],
    };
  }
</script>

{#snippet list(items: Turn[], found: boolean)}
  <ol class="who">
    {#each items as it (it.artist.id)}
      <li>
        <Cover kind="artist" id={it.artist.id} name={it.artist.name} size={28} />
        <div class="name">
          <a href={entityHref("artist", it.artist.id)}>{it.artist.name}</a>
          <span class="muted small">{found ? t("disc.since", { date: date(it.first_ts) }) : t("disc.last", { date: date(it.last_ts) })}</span>
        </div>
        <span class="mono muted small" title={t(found ? "disc.burstFound" : "disc.burstDropped", { n: it.burst })}>
          {num(it.burst)} / {num(it.artist.plays)}
        </span>
      </li>
    {/each}
  </ol>
{/snippet}

<section class="panel">
  <h2>{t("disc.title")}</h2>
  <p class="muted small">{t("disc.help")}</p>
  {#if d && (d.found.length || d.dropped.length || d.per_month.length)}
    {#if d.per_month.length > 1}
      <h3>{t("disc.perMonth")}</h3>
      <Chart option={perMonth} height="160px" label={t("disc.perMonth")} />
    {/if}
    <div class="two">
      <div>
        <h3>{t("disc.found")}</h3>
        {#if d.found.length}{@render list(d.found, true)}{:else}<p class="muted">{t("disc.none")}</p>{/if}
      </div>
      <div>
        <h3>{t("disc.dropped")}</h3>
        {#if d.dropped.length}{@render list(d.dropped, false)}{:else}<p class="muted">{t("disc.none")}</p>{/if}
      </div>
    </div>
  {:else if d}
    <p class="muted">{t("disc.none")}</p>
  {/if}
</section>

<style>
  .two { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 16px 28px; }
  h3 { font-family: var(--font-body); font-weight: 500; font-size: 13.5px; margin: 0 0 4px; color: var(--fg-2); }
  p { margin: 0; }
  .who { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .who li { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 10px; align-items: center; }
  .name { display: grid; min-width: 0; }
  .name a { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
