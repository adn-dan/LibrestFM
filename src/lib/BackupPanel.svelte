<script lang="ts">
  // Sauvegarde et restauration de toute la base (historique, nettoyage, enrichissement).
  import { readable, shownPath, writeTo } from "$lib/androidFiles";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, type BackupInfo } from "./api";
  import { date, errText } from "./format";
  import { locale, t } from "./settings.svelte";

  let info = $state<{ dir: string; auto: [string, number][] } | null>(null);
  let message = $state("");
  let error = $state("");
  let busy = $state(false);
  /** Sauvegarde choisie pour la restauration, en attente de confirmation. */
  let pending = $state<{ path: string; info: BackupInfo } | null>(null);

  const refresh = async () => (info = await api.backups());
  $effect(() => {
    refresh().catch(() => {});
  });

  /** Taille dans l'unité et la langue choisies (« 47 Mo », « 47 MB »…). */
  const mb = (bytes: number) =>
    new Intl.NumberFormat(locale(), { style: "unit", unit: "megabyte", maximumFractionDigits: 1 }).format(bytes / 1e6);
  const summary = (b: BackupInfo) => t("bk.summary", { n: b.plays, from: date(b.first_ts), to: date(b.last_ts), size: mb(b.bytes) });

  async function backup() {
    const today = new Date().toISOString().slice(0, 10);
    const path = await save({ defaultPath: `librestfm-${today}.sqlite`, filters: [{ name: "SQLite", extensions: ["sqlite"] }] });
    if (!path) return;
    busy = true;
    error = message = "";
    try {
      const name = `librestfm-${today}.sqlite`;
      const b = await writeTo(path, name, (p) => api.backupDb(p));
      message = t("bk.saved", { path: shownPath(path, name), summary: summary(b) });
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function chooseRestore() {
    const picked = await open({ filters: [{ name: "SQLite", extensions: ["sqlite", "db"] }], defaultPath: info?.dir });
    if (typeof picked !== "string") return;
    error = message = "";
    try {
      const path = await readable(picked);
      pending = { path, info: await api.checkBackup(path) };
    } catch (e) {
      error = errText(e);
    }
  }

  async function restore() {
    if (!pending) return;
    busy = true;
    try {
      await api.restoreDb(pending.path);
      // Tout l'affichage repart de la base restaurée.
      location.reload();
    } catch (e) {
      error = errText(e);
      busy = false;
    }
  }
</script>

<section class="panel">
  <h2>{t("bk.title")}</h2>
  <p class="muted small">{t("bk.help")}</p>
  <div class="row">
    <button class="btn" onclick={backup} disabled={busy}>{t("bk.save")}</button>
    <button class="btn" onclick={chooseRestore} disabled={busy}>{t("bk.restore")}</button>
  </div>
  {#if pending}
    <div class="confirm">
      <p>{t("bk.confirm", { summary: summary(pending.info) })}</p>
      <div class="row">
        <button class="btn danger" onclick={restore} disabled={busy}>{t("bk.confirmYes")}</button>
        <button class="btn" onclick={() => (pending = null)} disabled={busy}>{t("src.cancel")}</button>
      </div>
    </div>
  {/if}
  {#if message}<p class="small muted">{message}</p>{/if}
  {#if error}<p class="small err" role="alert">{error}</p>{/if}
  {#if info}
    <p class="small muted">
      {#if info.auto.length}
        {t("bk.auto", { n: info.auto.length })}
        <button class="link" onclick={() => revealItemInDir(info!.auto[0][0]).catch((e) => (error = errText(e)))}>{t("bk.openDir")}</button>
      {:else}
        {t("bk.autoNone")}
      {/if}
    </p>
  {/if}
</section>

<style>
  p { margin: 0; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; }
  .row .btn { padding: 4px 12px; font-size: 13px; }
  .confirm { display: grid; gap: 8px; padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--chip); }
  .err { color: var(--danger); }
  .link { background: none; border: 0; color: var(--accent); padding: 0; font-size: inherit; cursor: pointer; }
</style>
