// Libellés des rôles MusicBrainz et de la langue des paroles.
import { fr, type Key } from "./locales/fr";
import { lang, locale, t } from "./settings.svelte";
import { translateDetail } from "./instruments";

/** « producteur », « guitar » pour un instrument, « lead vocals » pour une voix. */
export function roleLabel(role: string, detail = ""): string {
  const d = detail ? translateDetail(detail, lang()) : "";
  // Pour un instrument ou une voix, la précision est plus parlante que le rôle.
  if ((role === "instrument" || role === "vocal") && d) return d;
  const key = `role.${role.replace(/ /g, "_")}`;
  const label = key in fr ? t(key as Key) : role;
  return d ? `${label} (${d})` : label;
}

/** « producer » ou « instrument: guitar », tel que renvoyé pour la page personne. */
export function roleFromPair(pair: string): string {
  const i = pair.indexOf(": ");
  return i < 0 ? roleLabel(pair) : roleLabel(pair.slice(0, i), pair.slice(i + 2));
}

// ISO 639-3 (MusicBrainz) → ISO 639-1, pour les langues courantes.
const ISO3: Record<string, string> = {
  eng: "en", fra: "fr", fre: "fr", spa: "es", deu: "de", ger: "de", ita: "it", por: "pt", jpn: "ja", kor: "ko",
  zho: "zh", chi: "zh", rus: "ru", nld: "nl", dut: "nl", swe: "sv", nor: "no", dan: "da", fin: "fi", isl: "is",
  ice: "is", pol: "pl", tur: "tr", ara: "ar", heb: "he", hin: "hi", ell: "el", gre: "el", ces: "cs", cze: "cs",
  hun: "hu", ron: "ro", rum: "ro", ukr: "uk", kat: "ka", lat: "la", gle: "ga", cym: "cy", wel: "cy", bam: "bm",
  wol: "wo", yor: "yo", swa: "sw", kea: "kea",
};

/** Langue(s) des paroles lisible(s) ; accepte une liste « eng,fra ». null si inconnue. */
export function lyricsLanguage(code: string | null): string | null {
  if (!code) return null;
  if (code.includes(",")) {
    const names = code.split(",").map(lyricsLanguage).filter((x): x is string => !!x);
    return names.length ? names.join(", ") : null;
  }
  if (code === "mul") return null;
  if (code === "zxx") return t("lyrics.none");
  try {
    return new Intl.DisplayNames(locale(), { type: "language" }).of(ISO3[code] ?? code) ?? code;
  } catch {
    return code;
  }
}
