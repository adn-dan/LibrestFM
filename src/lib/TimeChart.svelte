<script lang="ts">
  // Courbe des écoutes dans le temps, précision adaptée au zoom : par mois en
  // vue large, par semaine entre ~4 mois et ~2,5 ans visibles, par jour en dessous.
  // `range` cadre le zoom (période choisie) sans retirer de données : on peut
  // toujours dézoomer à la molette.
  import { echarts, chartLocale } from "./echarts";
  import { onMount, untrack } from "svelte";
  import { axisStyle, base, themeColors } from "./theme";
  import { settings, t, locale, lang } from "./settings.svelte";
  import { rangeOf } from "./drill.svelte";
  import { day, num } from "./format";

  type Pt = [number, number];
  type Grain = "month" | "week" | "day";

  let {
    perMonth,
    perDay,
    range = null,
    height = "260px",
    onpick,
  }: {
    perMonth: [string, number][];
    perDay: [string, number][];
    range?: [number, number] | null;
    height?: string;
    /** Clic sur la courbe : période (secondes) du mois, de la semaine ou du jour visé, et son libellé. */
    onpick?: (from: number, to: number, label: string) => void;
  } = $props();

  const DAY = 86_400_000;
  const grainFor = (span: number): Grain => (span > 900 * DAY ? "month" : span > 120 * DAY ? "week" : "day");
  const fmt: Record<Grain, (ms: number) => string> = {
    month: (ms) => new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" }).format(ms),
    week: (ms) => t("time.weekOf", { date: day(new Date(ms), true) }),
    day: (ms) => `${new Intl.DateTimeFormat(locale(), { weekday: "long" }).format(ms)} ${day(new Date(ms), true)}`,
  };

  let el: HTMLDivElement;
  let chart = $state.raw<echarts.ECharts>();
  let grain = $state<Grain>("month");
  let colors = $state(themeColors());

  const toTime = (s: string) => {
    const [y, m, d] = s.split("-").map(Number);
    return new Date(y, m - 1, d ?? 1).getTime();
  };
  const points = $derived.by(() => {
    const day: Pt[] = perDay.map(([k, n]) => [toTime(k), n]);
    // Semaines du lundi au dimanche, calculées à partir des jours.
    const weeks = new Map<number, number>();
    for (const [t, n] of day) {
      const d = new Date(t);
      const monday = new Date(d.getFullYear(), d.getMonth(), d.getDate() - ((d.getDay() + 6) % 7)).getTime();
      weeks.set(monday, (weeks.get(monday) ?? 0) + n);
    }
    return {
      month: perMonth.map(([k, n]) => [toTime(k), n] as Pt),
      week: [...weeks.entries()].sort((a, b) => a[0] - b[0]) as Pt[],
      day,
    };
  });
  const fullSpan = $derived(points.day.length ? points.day[points.day.length - 1][0] - points.day[0][0] : 0);

  function series() {
    const pts = points[grain];
    const c = colors;
    return [
      {
        type: "line" as const,
        data: pts,
        showSymbol: pts.length <= 2,
        // Lissage léger. « monotone » empêche la courbe de dépasser les points :
        // pas de faux pic entre deux valeurs, pas de passage sous zéro.
        smooth: 0.3,
        smoothMonotone: "x" as const,
        symbolSize: 8,
        lineStyle: { color: c.series[0], width: grain === "day" ? 1.5 : 2 },
        itemStyle: { color: c.series[0] },
        areaStyle: { color: c.series[0], opacity: 0.12 },
        sampling: "lttb" as const,
      },
    ];
  }

  function option() {
    const c = colors;
    return {
      ...base(c),
      grid: { left: 44, right: 16, top: 16, bottom: 56 },
      // Pas d'animation de mise à jour : sinon ECharts fait glisser l'ancienne
      // courbe vers la nouvelle et les deux se superposent pendant le changement de précision.
      animationDurationUpdate: 0,
      tooltip: {
        ...base(c).tooltip,
        trigger: "axis" as const,
        axisPointer: { type: "line" as const, lineStyle: { color: c.fg3 } },
        formatter: (p: any) => {
          // Pendant un changement de précision, l'infobulle peut viser un point absent.
          const v = p?.[0]?.value;
          if (!Array.isArray(v)) return "";
          return `${fmt[grain](v[0])}<br><b>${t("plays", { n: v[1] })}</b>`;
        },
      },
      xAxis: { type: "time" as const, ...axisStyle(c), splitLine: { show: false } },
      yAxis: { type: "value" as const, minInterval: 1, ...axisStyle(c), axisLabel: { ...axisStyle(c).axisLabel, formatter: (v: number) => num(v) } },
      dataZoom: [
        { type: "slider" as const, height: 18, bottom: 8, borderColor: c.line, textStyle: { color: c.fg3 }, filterMode: "none" as const },
        { type: "inside" as const, filterMode: "none" as const },
      ],
      series: series(),
    };
  }

  /** Plage visible actuelle, en millisecondes. */
  function visibleSpan(): number {
    const dz = (chart?.getOption() as any)?.dataZoom?.[0];
    if (!dz) return fullSpan;
    if (dz.startValue != null && dz.endValue != null) return dz.endValue - dz.startValue;
    return (fullSpan * ((dz.end ?? 100) - (dz.start ?? 0))) / 100;
  }

  /** Plage visible [début, fin] en ms, lue sur l'axe des temps. */
  function visibleRange(): [number, number] | null {
    const ext = (chart as any)?.getModel?.()?.getComponent("xAxis", 0)?.axis?.scale?.getExtent?.();
    return Array.isArray(ext) && Number.isFinite(ext[0]) ? [ext[0], ext[1]] : null;
  }

  /** Arrondi vers le haut à une valeur ronde (1, 1,2, 1,5, 2, 2,5, 3, 4, 5, 6, 8 × 10ⁿ). */
  function niceCeil(x: number): number {
    const mag = 10 ** Math.floor(Math.log10(x));
    return ([1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10].find((k) => k * mag >= x) ?? 10) * mag;
  }

  /** Échelle verticale calée sur les points visibles (le zoom ne filtre pas les données). */
  function fitY() {
    const r = visibleRange();
    const pts = points[grain];
    if (!r || !pts.length) return;
    let max = 0;
    for (const [x, n] of pts) if (x >= r[0] && x <= r[1] && n > max) max = n;
    chart?.setOption({ yAxis: { max: max > 0 ? niceCeil(max * 1.05) : null } });
  }

  function refreshGrain() {
    const want = grainFor(visibleSpan());
    if (want !== grain) {
      grain = want;
      // Ne remplace que la série : le zoom en cours est conservé.
      chart?.setOption({ series: series() }, { replaceMerge: ["series"] });
    }
    fitY();
  }

  // La langue des dates de l'axe se fixe à la création : on recrée le graphe quand elle change.
  $effect(() => {
    const loc = chartLocale();
    untrack(() => {
      chart?.dispose();
      const c = echarts.init(el, null, { renderer: "canvas", locale: loc });
      c.on("datazoom", refreshGrain);
      // Clic n'importe où dans la zone de tracé : point le plus proche à la précision affichée.
      c.getZr().on("click", (e: any) => {
        if (!onpick || !c.containPixel("grid", [e.offsetX, e.offsetY])) return;
        const x = c.convertFromPixel({ seriesIndex: 0 }, [e.offsetX, e.offsetY])?.[0];
        const pts = points[grain];
        if (x == null || !pts.length) return;
        const near = pts.reduce((a, b) => (Math.abs(b[0] - x) < Math.abs(a[0] - x) ? b : a));
        const { from, to } = rangeOf(near[0], grain);
        c.dispatchAction({ type: "hideTip" });
        onpick(from, to, fmt[grain](near[0]));
      });
      chart = c;
    });
  });

  // Thème forcé dans les Paramètres : relire les couleurs (l'attribut est déjà posé).
  $effect(() => {
    void settings.theme;
    void settings.accent;
    colors = themeColors();
  });

  onMount(() => {
    const ro = new ResizeObserver(() => chart?.resize());
    ro.observe(el);
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const onTheme = () => (colors = themeColors());
    mq.addEventListener("change", onTheme);
    return () => {
      ro.disconnect();
      mq.removeEventListener("change", onTheme);
      chart?.dispose();
    };
  });

  // Données, thème ou période changés : on redessine puis on cadre sur la période.
  $effect(() => {
    if (!chart) return;
    // Langue : libellés et infobulles.
    const deps = [colors, points, range, lang()];
    untrack(() => {
      grain = grainFor(range ? range[1] - range[0] : fullSpan);
      chart?.setOption(option(), true);
      if (range) chart?.dispatchAction({ type: "dataZoom", startValue: range[0], endValue: range[1] });
      fitY();
    });
    void deps;
  });
</script>

<div class="wrap">
  <div class="chart" class:pickable={!!onpick} bind:this={el} style:height role="img" aria-label={t("time.aria")}></div>
  <p class="muted small hint">
    {t(`time.${grain}` as const)}{lang() === "zh" || lang() === "ja" ? "。" : ". "}{grain === "day" ? t("time.zoomOut") : t("time.zoomIn")}
    {#if onpick}{t("dr.clickHint")}{/if}
  </p>
</div>

<style>
  .wrap { display: grid; gap: 4px; min-width: 0; }
  .chart { width: 100%; min-width: 0; }
  .pickable { cursor: pointer; }
  .hint { margin: 0; }
</style>
