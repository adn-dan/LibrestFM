<script lang="ts">
  // Carte du monde des écoutes par pays d'origine des artistes (tracés Natural Earth embarqués).
  import { echarts } from "./echarts";
  import Chart from "./Chart.svelte";
  import { feature } from "topojson-client";
  import countries from "i18n-iso-countries";
  import world from "world-atlas/countries-110m.json";
  import { base, type ThemeColors } from "./theme";
  import { country } from "./format";
  import { t } from "./settings.svelte";

  let { rows, onpick }: { rows: [string, number, number][]; onpick?: (code: string) => void } = $props();

  /** Contour qui traverse la ligne de changement de date (Russie, Fidji) : ses longitudes
   *  négatives passent au-delà de 180°, sinon ECharts relie les deux bords de la carte par
   *  une bande horizontale. */
  function unwrapAntimeridian(g: any) {
    const polys: number[][][][] = g?.type === "Polygon" ? [g.coordinates] : g?.type === "MultiPolygon" ? g.coordinates : [];
    for (const poly of polys) {
      for (const ring of poly) {
        const lons = ring.map((p) => p[0]);
        if (Math.max(...lons) - Math.min(...lons) > 180) for (const p of ring) if (p[0] < 0) p[0] += 360;
      }
    }
  }

  // Tracés enregistrés une fois ; chaque pays est nommé par son code numérique (« 250 » = France).
  if (!echarts.getMap("lfm-world")) {
    const geo: any = feature(world as any, (world as any).objects.countries);
    geo.features = geo.features.filter((f: any) => f.id !== "010"); // sans l'Antarctique
    for (const f of geo.features) {
      f.properties = { name: String(f.id) };
      unwrapAntimeridian(f.geometry);
    }
    echarts.registerMap("lfm-world", geo);
  }

  function option(c: ThemeColors) {
    const data = rows
      .map(([code, plays, artists]) => ({ name: countries.alpha2ToNumeric(code) ?? "", value: plays, code, artists }))
      .filter((d) => d.name);
    const max = Math.max(1, ...data.map((d) => d.value));
    return {
      ...base(c),
      tooltip: {
        ...base(c).tooltip,
        formatter: (p: any) =>
          p.data ? `<b>${country(p.data.code)}</b><br>${t("plays", { n: p.data.value })}<br>${t("orig.artists", { n: p.data.artists })}` : "",
      },
      visualMap: { min: 0, max, show: false, inRange: { color: c.seq.slice(1) } },
      series: [
        {
          type: "map" as const,
          map: "lfm-world",
          roam: true,
          data,
          itemStyle: { areaColor: c.surface, borderColor: c.line },
          emphasis: { label: { show: false }, itemStyle: { areaColor: c.series[1] } },
          select: { disabled: true },
        },
      ],
    };
  }
</script>

<Chart {option} height="340px" label={t("orig.title")} onclick={(p: any) => p.data?.code && onpick?.(p.data.code)} />
