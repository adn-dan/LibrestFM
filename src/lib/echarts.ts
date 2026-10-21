// ECharts avec la traduction française enregistrée (noms de mois et de jours
// des axes temporels). La langue d'un graphe se fixe à sa création.
// Seuls les graphiques et composants utilisés sont chargés (moitié moins lourd
// que la bibliothèque entière).
import * as echarts from "echarts/core";
import { BarChart, HeatmapChart, LineChart, MapChart, RadarChart } from "echarts/charts";
import {
  DataZoomComponent,
  GeoComponent,
  GridComponent,
  LegendComponent,
  RadarComponent,
  TooltipComponent,
  VisualMapComponent,
} from "echarts/components";
import { CanvasRenderer, SVGRenderer } from "echarts/renderers";
// @ts-expect-error fichier de traduction sans déclaration de types
import langFR from "echarts/i18n/langFR-obj.js";
// @ts-expect-error idem
import langES from "echarts/i18n/langES-obj.js";
// @ts-expect-error idem
import langDE from "echarts/i18n/langDE-obj.js";
// @ts-expect-error idem
import langIT from "echarts/i18n/langIT-obj.js";
// @ts-expect-error idem
import langPT from "echarts/i18n/langPT-br-obj.js";
// @ts-expect-error idem
import langZH from "echarts/i18n/langZH-obj.js";
// @ts-expect-error idem
import langJA from "echarts/i18n/langJA-obj.js";
import { lang } from "./settings.svelte";

echarts.use([
  BarChart,
  HeatmapChart,
  LineChart,
  MapChart,
  RadarChart,
  DataZoomComponent,
  GeoComponent,
  GridComponent,
  LegendComponent,
  RadarComponent,
  TooltipComponent,
  VisualMapComponent,
  CanvasRenderer,
  SVGRenderer,
]);
echarts.registerLocale("FR", langFR);
echarts.registerLocale("ES", langES);
echarts.registerLocale("DE", langDE);
echarts.registerLocale("IT", langIT);
echarts.registerLocale("PT", langPT);
echarts.registerLocale("ZH", langZH);
echarts.registerLocale("JA", langJA);

export const chartLocale = () => (lang() === "en" ? "EN" : lang().toUpperCase());
export { echarts };
