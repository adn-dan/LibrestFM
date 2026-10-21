// Synchronisation des comptes en ligne : état partagé par la page Sources et la barre latérale.
import { listen } from "@tauri-apps/api/event";
import { api, type SyncStatus } from "./api";
import { refreshJobs } from "./enrich.svelte";

export const sync = $state<{ st: SyncStatus | null; error: string; done: number }>({ st: null, error: "", done: 0 });

let watching = false;

export function watchSync() {
  if (watching) return;
  watching = true;
  api.syncStatus().then((s) => {
    sync.st = s;
  }).catch(() => {});
  listen<SyncStatus>("sync-progress", (e) => {
    const was = sync.st?.running;
    sync.st = e.payload;
    if (e.payload.running) sync.error = "";
    // Fin de synchronisation : de nouveaux titres à enrichir, des pages à relire.
    if (was && !e.payload.running) {
      sync.done++;
      refreshJobs();
    }
  });
  listen<string>("sync-error", (e) => (sync.error = e.payload));
}
