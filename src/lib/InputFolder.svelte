<script lang="ts">
  // Dossier d'import : lu à chaque démarrage (fichiers nouveaux ou modifiés).
  import { errText } from "./format";
  import { open } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, type InboxReport } from "./api";
  import { t } from "./settings.svelte";

  /** Appelé quand de nouvelles écoutes sont arrivées. */
  let { onchange }: { onchange?: () => void } = $props();

  let dir = $state("");
  let busy = $state(false);
  let last = $state<InboxReport | null>(null);
  let error = $state("");

  function done(r: InboxReport) {
    last = r;
    if (r.files.some((f) => f.added > 0)) onchange?.();
  }

  $effect(() => {
    api.inputDir().then((d) => (dir = d)).catch((e) => (error = errText(e)));
    // Scan du démarrage terminé après l'ouverture de la page.
    const un = listen<InboxReport>("inbox-done", (e) => done(e.payload));
    return () => void un.then((f) => f());
  });

  async function run(task: () => Promise<InboxReport>) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      done(await task());
      dir = await api.inputDir();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function choose() {
    const picked = await open({ directory: true, defaultPath: dir || undefined });
    if (typeof picked === "string") run(() => api.setInputDir(picked));
  }

  const added = (r: InboxReport) => r.files.reduce((s, f) => s + f.added, 0);
</script>

<section class="panel">
  <h2>{t("in.title")}</h2>
  <p class="muted small">{t("in.help")}</p>
  <div class="row">
    <code class="path" title={dir}>{dir}</code>
    <button class="btn" onclick={() => revealItemInDir(dir).catch((e) => (error = errText(e)))} disabled={!dir}>{t("in.open")}</button>
    <button class="btn" onclick={choose} disabled={busy}>{t("in.change")}</button>
    <button class="btn" onclick={() => run(api.scanInput)} disabled={busy}>{busy ? t("in.scanning") : t("in.scan")}</button>
  </div>
  {#if last}
    <p class="small muted">
      {#if last.files.length}
        {t("in.result", { files: last.files.length, n: added(last) })}
      {:else}
        {t("in.nothing", { n: last.unchanged })}
      {/if}
    </p>
  {/if}
  {#if error}<p class="err small" role="alert">{error}</p>{/if}
</section>

<style>
  p { margin: 0; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .path { flex: 1 1 260px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); font-size: 13px; background: var(--chip); border-radius: 6px; padding: 6px 10px; }
  .row .btn { padding: 4px 12px; font-size: 13px; }
  .err { color: var(--danger); }
</style>
