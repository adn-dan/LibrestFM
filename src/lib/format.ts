import { locale, lang, settings, t } from "./settings.svelte";
import { fr, type Key } from "./locales/fr";

/** Message d'erreur affichable : les erreurs du cœur arrivent en « code␟détail ». */
/** Texte inséré dans du HTML (infobulles des graphiques) : noms d'artistes et de titres échappés. */
export function esc(s: string | null | undefined): string {
  return (s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

export function errText(e: unknown): string {
  const s = String(e).replace(/^Error: /, "");
  const i = s.indexOf("\u001f");
  if (i < 0) return s;
  const key = `err.${s.slice(0, i)}`;
  return key in fr ? t(key as Key, { detail: s.slice(i + 1) }) : s.slice(i + 1);
}

export const num = (n: number) => new Intl.NumberFormat(locale()).format(n);
/** Un jour, au format choisi dans les Paramètres (selon la langue par défaut ; `long` : mois en toutes lettres). */
export function day(d: Date, long = false, format = settings.dateFormat): string {
  const p = (n: number) => String(n).padStart(2, "0");
  const [y, m, j] = [d.getFullYear(), p(d.getMonth() + 1), p(d.getDate())];
  switch (format) {
    case "dmy":
      return `${j}/${m}/${y}`;
    case "mdy":
      return `${m}/${j}/${y}`;
    case "ymd":
      return `${y}-${m}-${j}`;
    default:
      return new Intl.DateTimeFormat(locale(), { day: "numeric", month: long ? "long" : "short", year: "numeric" }).format(d);
  }
}

export const date = (ts: number | null) => (ts == null ? "–" : day(new Date(ts * 1000)));

/** « 2024-03 » → « mars 2024 » / « Mar 2024 » */
export function month(ym: string): string {
  const [y, m] = ym.split("-").map(Number);
  return new Intl.DateTimeFormat(locale(), { month: "short", year: "numeric" }).format(new Date(y, m - 1, 1));
}

/** Durée : « 3:52 » pour un titre, « 1 h 11 min » au-delà d'une heure. */
export function duration(seconds: number | null | undefined): string {
  if (seconds == null) return "–";
  const s = Math.round(seconds);
  if (s < 3600) return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  return `${Math.floor(s / 3600)} h ${String(Math.round((s % 3600) / 60)).padStart(2, "0")} min`;
}

/** Durée longue en minutes : « 29 min », « 1 h 52 min ». */
export function minutes(total: number | null | undefined): string {
  if (total == null) return "–";
  const m = Math.round(total);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} min`;
}

/** Temps d'écoute cumulé : « 3 273 h », « 4 h 12 min ». */
export function hours(seconds: number): string {
  return seconds >= 10 * 3600 ? `${num(Math.round(seconds / 3600))} h` : minutes(seconds / 60);
}

/** Date ISO éventuellement partielle : « 2004-02-26 », « 2004-02 » ou « 2004 ». */
export function partialDate(d: string | null | undefined): string {
  if (!d) return "–";
  const [y, m, dd] = d.split("-").map(Number);
  if (!m) return String(y);
  if (dd) return day(new Date(y, m - 1, dd), true);
  return new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" }).format(new Date(y, m - 1, 1));
}

/** Nom du pays à partir du code ISO (FR → France). */
export function country(code: string | null | undefined): string {
  if (!code) return "–";
  try {
    return new Intl.DisplayNames(locale(), { type: "region" }).of(code) ?? code;
  } catch {
    return code;
  }
}

/** Noms des notes : solfège en langues romanes, lettres ailleurs (H pour si en allemand). */
const NOTES: Record<string, string[]> = {
  fr: ["Do", "Do♯", "Ré", "Ré♯", "Mi", "Fa", "Fa♯", "Sol", "Sol♯", "La", "La♯", "Si"],
  es: ["Do", "Do♯", "Re", "Re♯", "Mi", "Fa", "Fa♯", "Sol", "Sol♯", "La", "La♯", "Si"],
  it: ["Do", "Do♯", "Re", "Re♯", "Mi", "Fa", "Fa♯", "Sol", "Sol♯", "La", "La♯", "Si"],
  pt: ["Dó", "Dó♯", "Ré", "Ré♯", "Mi", "Fá", "Fá♯", "Sol", "Sol♯", "Lá", "Lá♯", "Si"],
  de: ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "H"],
  en: ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"],
  zh: ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"],
  ja: ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"],
};
export const notes = () => NOTES[lang()];

/** Axes du radar, dans l'ordre de `stats::AXES` côté Rust. */
const AXIS_IDS = ["danceability", "energy", "valence", "acousticness", "instrumentalness", "speechiness", "liveness"] as const;
export const axes = () =>
  AXIS_IDS.map((id) => ({ label: t(`axis.${id}` as Key), help: t(`axis.${id}.help` as Key) }));

/** Jours courts, lundi en premier. */
export const weekdays = () =>
  Array.from({ length: 7 }, (_, i) => new Intl.DateTimeFormat(locale(), { weekday: "short" }).format(new Date(2024, 0, 1 + i)));

/** Nom affiché d'une source : le journal du scrobbleur Android sous un nom parlant. */
export function sourceName(fileName: string): string {
  return fileName === "scrobbles.csv" ? t("src.phoneJournal") : fileName;
}
