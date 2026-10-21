// Paramètres de l'interface (langue, thème) et traduction. Gardés dans le
// stockage local de la fenêtre : propres à cet ordinateur, rien à synchroniser.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { fr, type Key } from "./locales/fr";
import { en } from "./locales/en";
import { es } from "./locales/es";
import { de } from "./locales/de";
import { it } from "./locales/it";
import { pt } from "./locales/pt";
import { zh } from "./locales/zh";
import { ja } from "./locales/ja";

export type Lang = "fr" | "en" | "es" | "de" | "it" | "pt" | "zh" | "ja";
/** Format des dates : selon la langue, ou numérique imposé. */
export const DATE_FORMATS = ["auto", "dmy", "mdy", "ymd"] as const;
export type DateFormat = (typeof DATE_FORMATS)[number];
export type LangChoice = Lang | "system";
export type Theme = "system" | "light" | "dark";
/** Couleur principale ; les teintes sont dans app.css (data-accent). */
export const ACCENTS = ["blue", "violet", "green", "orange", "rose"] as const;
export type Accent = (typeof ACCENTS)[number];

const DICTS: Record<Lang, Record<Key, string | { one: string; other: string }>> = { fr, en, es, de, it, pt, zh, ja };
export const LANGS: { id: Lang; name: string }[] = [
  { id: "fr", name: "Français" },
  { id: "en", name: "English" },
  { id: "es", name: "Español" },
  { id: "de", name: "Deutsch" },
  { id: "it", name: "Italiano" },
  { id: "pt", name: "Português" },
  { id: "zh", name: "中文（简体）" },
  { id: "ja", name: "日本語" },
];
/** Paramètre régional (dates, nombres) de chaque langue. */
const LOCALES: Record<Lang, string> = { fr: "fr-FR", en: "en-GB", es: "es-ES", de: "de-DE", it: "it-IT", pt: "pt-PT", zh: "zh-CN", ja: "ja-JP" };

function load<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key) as T | null;
    return v && allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

function save(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Stockage indisponible : le réglage vaut pour la session.
  }
}

const systemLang = (): Lang => {
  const code = navigator.language.toLowerCase().slice(0, 2);
  return LANGS.some((l) => l.id === code) ? (code as Lang) : "en";
};

/** Tailles de mix proposées (titres). */
export const MIX_SIZES = ["15", "20", "30", "40", "50"] as const;

export const settings = $state({
  langChoice: load<LangChoice>("lfm.lang", ["system", "fr", "en", "es", "de", "it", "pt", "zh", "ja"], "system"),
  dateFormat: load<DateFormat>("lfm.dateFormat", DATE_FORMATS, "auto"),
  theme: load<Theme>("lfm.theme", ["system", "light", "dark"], "system"),
  mixSize: Number(load("lfm.mixSize", MIX_SIZES, "30")),
  accent: load<Accent>("lfm.accent", ACCENTS, "blue"),
});

/** Langue effective. */
export const lang = (): Lang => (settings.langChoice === "system" ? systemLang() : settings.langChoice);
/** Paramètre régional pour les dates et les nombres. */
export const locale = () => LOCALES[lang()];

/** Texte traduit ; {x} remplacé par vars.x ; pluriel choisi selon vars.n. */
export function t(key: Key, vars: Record<string, string | number> = {}): string {
  const entry = DICTS[lang()][key] ?? fr[key];
  const text =
    typeof entry === "string" ? entry : entry[new Intl.PluralRules(locale()).select(Number(vars.n ?? 0)) === "one" ? "one" : "other"];
  return text.replace(/\{(\w+)\}/g, (_, k) => {
    const v = vars[k];
    return v == null ? "" : typeof v === "number" ? new Intl.NumberFormat(locale()).format(v) : v;
  });
}

function applyTheme() {
  const root = document.documentElement;
  if (settings.theme === "system") delete root.dataset.theme;
  else root.dataset.theme = settings.theme;
  // Barre de titre de la fenêtre assortie (null = suivre le système).
  getCurrentWindow()
    .setTheme(settings.theme === "system" ? null : settings.theme)
    .catch(() => {});
}

function applyLang() {
  document.documentElement.lang = lang();
}

export function setTheme(theme: Theme) {
  settings.theme = theme;
  save("lfm.theme", theme);
  applyTheme();
}

function applyAccent() {
  const root = document.documentElement;
  if (settings.accent === "blue") delete root.dataset.accent;
  else root.dataset.accent = settings.accent;
}

export function setAccent(accent: Accent) {
  settings.accent = accent;
  save("lfm.accent", accent);
  applyAccent();
}

export function setDateFormat(f: DateFormat) {
  settings.dateFormat = f;
  save("lfm.dateFormat", f);
}

export function setMixSize(n: number) {
  settings.mixSize = n;
  save("lfm.mixSize", String(n));
}

/** Taille choisie pour un mix donné (sinon la taille par défaut). */
export function mixSizeOf(key: string): number {
  try {
    const v = Number(JSON.parse(localStorage.getItem("lfm.mixSizes") ?? "{}")[key]);
    return MIX_SIZES.includes(String(v) as (typeof MIX_SIZES)[number]) ? v : settings.mixSize;
  } catch {
    return settings.mixSize;
  }
}

export function setMixSizeOf(key: string, n: number) {
  try {
    const all = JSON.parse(localStorage.getItem("lfm.mixSizes") ?? "{}");
    all[key] = n;
    localStorage.setItem("lfm.mixSizes", JSON.stringify(all));
  } catch {
    // Stockage indisponible : le choix vaut pour la session.
  }
}

export function setLang(choice: LangChoice) {
  settings.langChoice = choice;
  save("lfm.lang", choice);
  applyLang();
}

/** À appeler au démarrage. */
export function applySettings() {
  applyTheme();
  applyAccent();
  applyLang();
}
