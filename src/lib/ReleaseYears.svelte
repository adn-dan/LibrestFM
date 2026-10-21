<script lang="ts">
  // Écoutes par année (ou décennie) de sortie des albums ; un clic sur une barre
  // ouvre le détail de cette période de sortie.
  import type { Period } from "./api";
  import { openDrill } from "./drill.svelte";
  import Chart from "./Chart.svelte";
  import { axisStyle, base, type ThemeColors } from "./theme";
  import { t } from "./settings.svelte";

  let { years, covered, period, subtitle }: { years: [number, number][]; covered: number; period: Period; subtitle?: string } = $props();

  let byDecade = $state(false);

  /** Toutes les années (ou décennies) entre la plus ancienne et la plus récente, trous compris. */
  const bars = $derived.by(() => {
    if (!years.length) return [] as { from: number; to: number; label: string; plays: number }[];
    const step = byDecade ? 10 : 1;
    const start = Math.floor(years[0][0] / step) * step;
    const end = years[years.length - 1][0];
    const out = [];
    for (let y = start; y <= end; y += step) {
      const plays = years.filter(([yr]) => yr >= y && yr < y + step).reduce((s, [, n]) => s + n, 0);
      out.push({ from: y, to: y + step - 1, label: byDecade ? t("rel.decade", { d: String(y) }) : String(y), plays });
    }
    return out;
  });

  function option(c: ThemeColors) {
    return {
      ...base(c),
      grid: { left: 44, right: 16, top: 12, bottom: 28 },
      tooltip: { ...base(c).tooltip, trigger: "axis" as const, formatter: (p: any) => `${p[0].name}<br><b>${t("plays", { n: p[0].value })}</b>` },
      xAxis: { type: "category" as const, data: bars.map((b) => b.label), ...axisStyle(c) },
      yAxis: { type: "value" as const, minInterval: 1, ...axisStyle(c) },
      series: [
        {
          type: "bar" as const,
          data: bars.map((b) => ({ value: b.plays, itemStyle: { color: c.series[0], borderRadius: [3, 3, 0, 0] } })),
          barMaxWidth: 28,
          cursor: "pointer",
        },
      ],
    };
  }

  function pick(i: number) {
    const b = bars[i];
    if (!b || !b.plays) return;
    openDrill(t("rel.albums", { label: b.label }), { ...period, release_from: b.from, release_to: b.to }, { top: 30, subtitle: subtitle ?? "" });
  }
</script>

<section class="panel">
  <div class="panel-head">
    <h2>{t("rel.title")}</h2>
    <div class="seg" role="group" aria-label={t("rel.title")}>
      <button aria-pressed={!byDecade} onclick={() => (byDecade = false)}>{t("rel.years")}</button>
      <button aria-pressed={byDecade} onclick={() => (byDecade = true)}>{t("rel.decades")}</button>
    </div>
  </div>
  {#if years.length}
    <Chart option={option} height="240px" label={t("rel.title")} onclick={(p) => pick(p.dataIndex)} />
    <p class="muted small">{t("rel.covered", { p: Math.round(covered * 100) })} {t("rel.hint")}</p>
  {:else}
    <p class="muted">{t("rel.none")}</p>
  {/if}
</section>

<style>
  p { margin: 0; }
</style>
