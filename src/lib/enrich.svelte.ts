// Tâches de fond (caractéristiques ReccoBeats, crédits MusicBrainz) : état
// partagé par la page Enrichissement et l'indicateur de la barre latérale.
import { errText } from "./format";
import { listen } from "@tauri-apps/api/event";
import { api, type CreditsStatus, type EnrichStatus } from "./api";

/** État commun aux deux tâches. */
export interface JobStatus {
  ok: number;
  not_found: number;
  errors: number;
  pending: number;
  plays_covered: number;
  running: boolean;
  /** Élément en cours (artiste, ou « artiste – titre »). */
  current: string | null;
  /** Pause imposée par le service : heure Unix de reprise. */
  throttled_until: number | null;
}

interface JobDef<S> {
  event: string;
  status: () => Promise<S>;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  retry: () => Promise<number>;
  normalize: (s: S) => JobStatus;
}

export const clock = $state({ now: Date.now() / 1000 });

function makeJob<S>(def: JobDef<S>) {
  const job = $state<{ st: JobStatus | null; error: string }>({ st: null, error: "" });
  const refresh = async () => {
    try {
      const fresh = def.normalize(await def.status());
      // L'état relu ne connaît pas l'élément en cours : on garde le dernier annoncé.
      job.st = { ...fresh, current: fresh.running ? (job.st?.current ?? null) : null };
    } catch (e) {
      job.error = errText(e);
    }
  };
  return {
    get st() {
      return job.st;
    },
    get error() {
      return job.error;
    },
    refresh,
    async start() {
      job.error = "";
      await def.start();
      if (job.st) job.st = { ...job.st, running: true };
    },
    stop: () => def.stop(),
    /** Remet les échecs en attente et relance aussitôt la récupération. */
    async retry() {
      await def.retry();
      job.error = "";
      await def.start();
      await refresh();
      if (job.st) job.st = { ...job.st, running: true };
    },
    /** Secondes restantes d'une pause imposée (0 s'il n'y en a pas). */
    throttleLeft() {
      const until = job.st?.throttled_until;
      return until && until > clock.now ? Math.ceil(until - clock.now) : 0;
    },
    watch() {
      refresh();
      listen<S>(`${def.event}-progress`, (e) => {
        job.st = def.normalize(e.payload);
        job.error = "";
      });
      listen<string>(`${def.event}-error`, (e) => {
        job.error = e.payload;
        refresh();
      });
      // Pendant la tâche, relecture régulière (pauses imposées entre deux éléments).
      setInterval(() => {
        if (job.st?.running) refresh();
      }, 2000);
    },
  };
}

export const features = makeJob<EnrichStatus>({
  event: "enrich",
  status: api.enrichStatus,
  start: api.startEnrich,
  stop: api.stopEnrich,
  retry: api.retryEnrichFailures,
  normalize: (s) => ({ ...s, current: s.current_artist }),
});

export const credits = makeJob<CreditsStatus>({
  event: "credits",
  status: api.creditsStatus,
  start: api.startCredits,
  stop: api.stopCredits,
  retry: api.retryCreditFailures,
  // Fiches à relire : comptées en attente (le bouton reste disponible).
  normalize: (s) => ({ ...s, pending: s.pending + s.refresh, throttled_until: null }),
});

export const infos = makeJob<CreditsStatus>({
  event: "info",
  status: api.infoStatus,
  start: api.startInfo,
  stop: api.stopInfo,
  retry: api.retryInfoFailures,
  normalize: (s) => ({ ...s, throttled_until: null }),
});

export type Job = typeof features;

/** Part de chaque statut dans le total, en %. */
export function shares(st: JobStatus) {
  const total = st.ok + st.not_found + st.errors + st.pending;
  const pct = (n: number) => (total ? (100 * n) / total : 0);
  return { ok: pct(st.ok), notFound: pct(st.not_found), errors: pct(st.errors) };
}

/** Récupération automatique : chaque tâche repart dès qu'il y a des données manquantes
 *  (nouvelle écoute, import, synchro). Réglage propre à cet appareil. */
export const auto = $state({ on: readAuto() });

function readAuto(): boolean {
  try {
    return localStorage.getItem("lfm.autoEnrich") === "1";
  } catch {
    return false;
  }
}

export function setAuto(on: boolean) {
  auto.on = on;
  try {
    localStorage.setItem("lfm.autoEnrich", on ? "1" : "0");
  } catch {
    // Stockage indisponible : réglage pour cette session seulement.
  }
  if (on) autoStart();
}

/** Dernier lancement automatique par tâche : une tâche en échec (réseau) attend 5 min. */
const lastAuto = new Map<Job, number>();

function autoStart() {
  if (!auto.on) return;
  const now = Date.now();
  for (const j of ALL()) {
    if (!j.st || j.st.running || j.st.pending <= 0) continue;
    if (j.error && now - (lastAuto.get(j) ?? 0) < 300_000) continue;
    lastAuto.set(j, now);
    j.start().catch(() => {});
  }
}

let watching = false;
/** À appeler une fois (layout). */
export function watchJobs() {
  if (watching) return;
  watching = true;
  setInterval(() => (clock.now = Date.now() / 1000), 1000);
  features.watch();
  credits.watch();
  infos.watch();
  // États relus toutes les 30 s : de nouveaux titres en attente lancent la récupération.
  setInterval(() => {
    if (!auto.on) return;
    Promise.all(ALL().map((j) => j.refresh())).then(autoStart);
  }, 30_000);
  setTimeout(() => Promise.all(ALL().map((j) => j.refresh())).then(autoStart), 3000);
}

const ALL = () => [features, credits, infos];

/** Une tâche au moins tourne. */
export const anyRunning = () => ALL().some((j) => j.st?.running);
/** Une tâche au moins a quelque chose à faire. */
export const anyPending = () => ALL().some((j) => (j.st?.pending ?? 0) > 0 && !j.st?.running);

/** Lance toutes les tâches qui ont des éléments en attente. */
export async function startAll() {
  await Promise.all(ALL().filter((j) => (j.st?.pending ?? 0) > 0 && !j.st?.running).map((j) => j.start()));
}

export async function stopAll() {
  await Promise.all(ALL().filter((j) => j.st?.running).map((j) => j.stop()));
}

/** Après un import ou un retrait de source : de nouveaux titres sont en attente. */
export function refreshJobs() {
  Promise.all(ALL().map((j) => j.refresh())).then(autoStart);
}
