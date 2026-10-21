// Couleurs lues depuis les variables CSS (app.css), pour que les graphes suivent le thème.
export interface ThemeColors {
  fg: string;
  fg2: string;
  fg3: string;
  line: string;
  surface: string;
  series: [string, string, string, string];
  /** Rampe séquentielle (bleu), du plus clair au plus foncé. */
  seq: string[];
  font: string;
}

export function themeColors(): ThemeColors {
  const s = getComputedStyle(document.documentElement);
  const v = (n: string) => s.getPropertyValue(n).trim();
  return {
    fg: v("--fg"),
    fg2: v("--fg-2"),
    fg3: v("--fg-3"),
    line: v("--line"),
    surface: v("--surface"),
    series: [v("--s1"), v("--s2"), v("--s3"), v("--s4")],
    seq: [v("--seq-0"), v("--seq-1"), v("--seq-2"), v("--seq-3"), v("--seq-4")],
    font: v("--font-body"),
  };
}

/** Réglages communs : axes discrets, texte aux couleurs du thème. */
export function base(c: ThemeColors) {
  return {
    textStyle: { fontFamily: c.font, color: c.fg2 },
    animationDuration: matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 300,
    tooltip: {
      backgroundColor: c.surface,
      borderColor: c.line,
      textStyle: { color: c.fg, fontFamily: c.font },
    },
  };
}

export function axisStyle(c: ThemeColors) {
  return {
    axisLine: { lineStyle: { color: c.line } },
    axisTick: { show: false },
    axisLabel: { color: c.fg3 },
    splitLine: { lineStyle: { color: c.line, type: "dashed" as const } },
  };
}


/** Histogramme horizontal : une barre par libellé, valeur affichée au bout. */
export function hbars(c: ThemeColors, labels: string[], values: number[], tip: (i: number) => string, fmt: (n: number) => string, labelWidth = 170) {
  return {
    ...base(c),
    grid: { left: 8, right: 56, top: 4, bottom: 4, containLabel: true },
    tooltip: { ...base(c).tooltip, formatter: (p: any) => tip(p.dataIndex) },
    xAxis: { type: "value" as const, show: false },
    yAxis: {
      type: "category" as const,
      inverse: true,
      data: labels,
      ...axisStyle(c),
      axisLine: { show: false },
      axisLabel: { color: c.fg2, width: labelWidth, overflow: "truncate" as const },
    },
    series: [
      {
        type: "bar" as const,
        data: values,
        barMaxWidth: 16,
        itemStyle: { color: c.series[0], borderRadius: [0, 3, 3, 0] },
        label: { show: true, position: "right" as const, color: c.fg3, formatter: (p: any) => fmt(p.value) },
      },
    ],
  };
}
