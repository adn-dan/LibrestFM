// Panneau de détail : n'importe quel graphique l'ouvre avec un titre et un filtre.
import type { Filter } from "./api";

export const drill = $state<{ open: boolean; title: string; subtitle: string; filter: Filter; timeline: boolean; top: number }>({
  open: false,
  title: "",
  subtitle: "",
  filter: {},
  timeline: true,
  top: 10,
});

/**
 * Ouvre le détail d'une sélection. `timeline` : afficher la courbe dans le temps
 * (inutile quand la sélection est déjà un jour ou un mois).
 */
export function openDrill(title: string, filter: Filter, opts: { subtitle?: string; timeline?: boolean; top?: number } = {}) {
  drill.top = opts.top ?? 10;
  drill.title = title;
  drill.subtitle = opts.subtitle ?? "";
  drill.filter = filter;
  drill.timeline = opts.timeline ?? true;
  drill.open = true;
}

export function closeDrill() {
  drill.open = false;
}

/** Période (secondes) d'un mois, d'une semaine ou d'un jour commençant à `startMs` (heure locale). */
export function rangeOf(startMs: number, grain: "month" | "week" | "day"): { from: number; to: number } {
  const d = new Date(startMs);
  const end =
    grain === "month"
      ? new Date(d.getFullYear(), d.getMonth() + 1, 1)
      : new Date(d.getFullYear(), d.getMonth(), d.getDate() + (grain === "week" ? 7 : 1));
  return { from: Math.floor(startMs / 1000), to: Math.floor(end.getTime() / 1000) - 1 };
}
