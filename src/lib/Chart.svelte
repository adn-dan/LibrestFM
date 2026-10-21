<script lang="ts">
  // Enveloppe ECharts : suit la taille du conteneur et le thème clair/sombre.
  // `option` reçoit une fonction des couleurs du thème pour que tout se redessine
  // quand le système change de thème.
  import { echarts } from "./echarts";
  import type { ECElementEvent, EChartsOption } from "echarts";
  import { onMount } from "svelte";
  import { themeColors, type ThemeColors } from "./theme";
  import { settings } from "./settings.svelte";

  let {
    option,
    height = "320px",
    label,
    onclick,
  }: {
    option: (c: ThemeColors) => EChartsOption;
    height?: string;
    label: string;
    onclick?: (p: ECElementEvent) => void;
  } = $props();

  let el: HTMLDivElement;
  let chart = $state<echarts.ECharts | undefined>();
  let colors = $state(themeColors());

  onMount(() => {
    chart = echarts.init(el, null, { renderer: "svg" });
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

  // Thème forcé dans les Paramètres : relire les couleurs (l'attribut est déjà posé).
  $effect(() => {
    void settings.theme;
    void settings.accent;
    colors = themeColors();
  });

  $effect(() => {
    if (!chart) return;
    chart.setOption(option(colors), true);
  });

  $effect(() => {
    if (!chart || !onclick) return;
    const c = chart;
    const user = onclick;
    // L'infobulle resterait affichée par-dessus le panneau de détail.
    const handler = (p: ECElementEvent) => {
      c.dispatchAction({ type: "hideTip" });
      user(p);
    };
    chart.on("click", handler);
    return () => {
      if (chart && !chart.isDisposed()) chart.off("click", handler);
    };
  });
</script>

<div class="chart" bind:this={el} style:height role="img" aria-label={label}></div>

<style>
  .chart {
    width: 100%;
    min-width: 0;
  }
</style>
