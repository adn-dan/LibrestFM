// Android : notifications facultatives (scrobbleur actif, avancement des téléchargements).
// Les textes partent d'ici, dans la langue de l'app ; le service les garde pour quand l'app est fermée.
import { api } from "./api";
import { credits, features, infos } from "./enrich.svelte";
import { t } from "./settings.svelte";
import { sync } from "./sync.svelte";

export function notifLabels(): Record<string, string> {
  return {
    activeTitle: t("notif.activeTitle"),
    idle: t("notif.idle"),
    // Repères gardés tels quels : le service y met le titre et l'artiste.
    playing: t("notif.playing", { track: "{track}", artist: "{artist}" }),
    jobsTitle: t("notif.jobsTitle"),
  };
}

/** Texte et pourcentage des téléchargements en cours ; null : rien en cours. */
export function jobsProgress(): { text: string; percent: number | null } | null {
  const parts: string[] = [];
  const shares: number[] = [];
  if (sync.st?.running && sync.st.label) {
    parts.push(sync.st.progress > 0 ? t("mini.sync", { label: sync.st.label, p: Math.round(sync.st.progress * 100) }) : t("mini.syncNew", { label: sync.st.label }));
    if (sync.st.progress > 0) shares.push(sync.st.progress);
  }
  for (const [job, label] of [[features, "mini.features"], [credits, "mini.credits"], [infos, "mini.info"]] as const) {
    if (!job.st?.running) continue;
    parts.push(`${t(label)} ${t("percent", { n: Math.round(job.st.plays_covered * 100) })}`);
    shares.push(job.st.plays_covered);
  }
  if (!parts.length) return null;
  return { text: parts.join("\n"), percent: shares.length ? Math.round((shares.reduce((a, b) => a + b, 0) / shares.length) * 100) : null };
}

let last = "";
let timer: ReturnType<typeof setTimeout> | null = null;

/** Envoie l'avancement au plus toutes les 2 s (Android limite la fréquence des mises à jour). */
export function pushProgress() {
  if (timer) return;
  timer = setTimeout(() => {
    timer = null;
    const p = jobsProgress();
    const key = p ? `${p.text}|${p.percent}` : "";
    if (key === last) return;
    last = key;
    api.notifProgress(p?.text ?? null, p?.percent ?? null).catch(() => {});
  }, 2000);
}
