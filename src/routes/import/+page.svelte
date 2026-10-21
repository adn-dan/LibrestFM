<script lang="ts">
  import { readable } from "$lib/androidFiles";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { api, type ImportReport, type SourceInfo } from "$lib/api";
  import { date, num, errText, sourceName } from "$lib/format";
  import { refreshJobs } from "$lib/enrich.svelte";
  import { t } from "$lib/settings.svelte";
  import type { Key } from "$lib/locales/fr";
  import Accounts from "$lib/Accounts.svelte";
  import InputFolder from "$lib/InputFolder.svelte";
  import { android } from "$lib/platform";
  import HistoryExport from "$lib/HistoryExport.svelte";

  const formatLabel = (code: string | null) => (code ? t(`format.${code}` as Key) : "");

  let sources = $state<SourceInfo[]>([]);
  let reports = $state<ImportReport[]>([]);
  let cleaned = $state(0);
  let busy = $state(false);
  let dragging = $state(false);
  let error = $state("");
  let confirmRemove = $state<number | null>(null);

  // Les comptes en ligne ont leur propre section.
  let version = $state(0);
  const refresh = async () => {
    sources = (await api.listSources()).filter((s) => !s.format.endsWith("_account"));
    version++;
  };

  async function importPaths(paths: string[]) {
    if (!paths.length || busy) return;
    busy = true;
    error = "";
    try {
      const r = await api.importFiles(paths);
      reports = r.reports;
      cleaned = r.cleaned;
      await refresh();
      refreshJobs(); // de nouveaux titres sont en attente
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function pick() {
    const sel = await open({
      multiple: true,
      filters: [
        { name: t("src.filter.exports"), extensions: ["csv", "tsv", "txt", "json", "jsonl"] },
        { name: t("src.filter.all"), extensions: ["*"] },
      ],
    });
    if (sel) importPaths(await Promise.all((Array.isArray(sel) ? sel : [sel]).map(readable)));
  }

  async function remove(id: number) {
    confirmRemove = null;
    await api.removeSource(id);
    await refresh();
    refreshJobs();
  }

  $effect(() => {
    refresh().catch((e) => (error = errText(e)));
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((ev) => {
        if (ev.payload.type === "over" || ev.payload.type === "enter") dragging = true;
        else if (ev.payload.type === "leave") dragging = false;
        else if (ev.payload.type === "drop") {
          dragging = false;
          importPaths(ev.payload.paths);
        }
      })
      .then((u) => (unlisten = u));
    return () => unlisten?.();
  });

</script>

<div class="page">
  <header>
    <div class="eyebrow">{t("nav.sources")}</div>
    <h1>{t("src.title")}</h1>
  </header>

  <section class="drop panel" class:dragging aria-busy={busy}>
    <p><b>{t("src.drop")}</b> {t("src.or")}</p>
    <button class="btn primary" onclick={pick} disabled={busy}>{busy ? t("src.importing") : t("src.choose")}</button>
    <p class="muted small">{t("src.formats")}</p>
  </section>

  <Accounts onchange={() => refresh()} />
  <!-- Android : pas de dossier d'import (le sélecteur de dossier n'y existe pas) ; le
       scrobbleur et « Choisir des fichiers » le remplacent. -->
  {#if !android}<InputFolder onchange={() => { refresh(); refreshJobs(); }} />{/if}

  {#if error}<p class="panel" role="alert">{t("error", { msg: error })}</p>{/if}

  {#if reports.length}
    <section class="panel">
      <h2>{t("src.latest")}</h2>
      {#if cleaned > 0}
        <p class="small">{t("src.cleaned", { n: cleaned })} <a href="/cleanup">{t("src.cleanedLink")}</a></p>
      {/if}
      {#each reports as r (r.file_name + r.source_id)}
        <article class="report">
          <div class="rhead">
            <b>{sourceName(r.file_name)}</b>
            <span class="pill" data-status={r.status}>{t(`src.status.${r.status}` as const)}</span>
            {#if r.format}<span class="muted small">{formatLabel(r.format)}</span>{/if}
          </div>
          {#if r.status === "imported"}
            <dl class="kpis">
              <div class="kpi"><dt>{t("src.rows")}</dt><dd>{num(r.rows_read)}</dd></div>
              <div class="kpi"><dt>{t("src.newPlays")}</dt><dd>{num(r.added)}</dd></div>
              <div class="kpi"><dt>{t("src.known")}</dt><dd>{num(r.merged)}</dd></div>
              <div class="kpi"><dt>{t("src.rejected")}</dt><dd>{num(r.rejected)}</dd></div>
              <div class="kpi"><dt>{t("src.period")}</dt><dd><small>{date(r.first_ts)} → {date(r.last_ts)}</small></dd></div>
            </dl>
            {#if r.rejects.length}
              <ul class="small">
                {#each r.rejects as x (x.reason)}
                  <li>{num(x.count)} × {t(`reject.${x.reason}` as Key)} <span class="muted">({t("src.lines", { lines: x.sample_lines.join(", ") + (x.count > x.sample_lines.length ? "…" : "") })})</span></li>
                {/each}
              </ul>
            {/if}
            {#if r.overlaps.length}
              <p class="small muted">
                {t("src.overlap", { list: r.overlaps.map((o) => t("src.overlapItem", { file: sourceName(o.file_name), n: o.shared })).join(", ") })}
              </p>
            {/if}
          {:else if r.status === "unrecognized"}
            <p class="small">{t("src.unrecognized")}</p>
          {/if}
        </article>
      {/each}
    </section>
  {/if}

  <section class="panel">
    <h2>{t("src.imported")}</h2>
    {#if sources.length}
      <div class="tablewrap">
        <table class="stack">
          <thead>
            <tr>
              <th>{t("src.col.file")}</th><th>{t("src.col.format")}</th><th>{t("src.period")}</th>
              <th class="num">{t("src.rows")}</th><th class="num">{t("src.col.added")}</th><th class="num">{t("src.col.merged")}</th>
              <th class="num">{t("src.rejected")}</th><th class="num">{t("src.col.unique")}</th><th></th>
            </tr>
          </thead>
          <tbody>
            {#each sources as s (s.id)}
              <tr>
                <td>{sourceName(s.file_name)}<div class="muted small">{t("src.importedOn", { date: date(s.imported_at) })}</div></td>
                <td class="small" data-label={t("src.col.format")}>{formatLabel(s.format)}</td>
                <td class="small" data-label={t("src.period")}>{date(s.first_ts)} → {date(s.last_ts)}</td>
                <td class="num" data-label={t("src.rows")}>{num(s.rows_read)}</td>
                <td class="num" data-label={t("src.col.added")}>{num(s.rows_added)}</td>
                <td class="num" data-label={t("src.col.merged")}>{num(s.rows_merged)}</td>
                <td class="num" data-label={t("src.rejected")}>{num(s.rows_rejected)}</td>
                <td class="num" title={t("src.uniqueTitle")} data-label={t("src.col.unique")}>{num(s.exclusive)}</td>
                <td>
                  {#if confirmRemove === s.id}
                    <span class="confirm">
                      {t("src.confirm", { n: s.exclusive })}
                      <button class="btn danger" onclick={() => remove(s.id)}>{t("src.remove")}</button>
                      <button class="btn" onclick={() => (confirmRemove = null)}>{t("src.cancel")}</button>
                    </span>
                  {:else}
                    <button class="btn" onclick={() => (confirmRemove = s.id)}>{t("src.remove")}</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="muted small">{t("src.uniqueNote")}</p>
    {:else}
      <p class="muted">{t("src.none")}</p>
    {/if}
  </section>

  <HistoryExport {version} />
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 1200px; }
  .drop { justify-items: center; text-align: center; border-style: dashed; border-width: 2px; padding: 32px; }
  .drop.dragging { border-color: var(--accent); background: var(--chip); }
  .drop p { margin: 0; max-width: 70ch; }
  .report { display: grid; gap: 8px; padding-block: 10px; border-top: 1px solid var(--line); }
  .report:first-of-type { border-top: 0; }
  .rhead { display: flex; flex-wrap: wrap; gap: 6px 12px; align-items: baseline; }
  .pill { font-size: 12px; border-radius: 999px; padding: 1px 9px; background: var(--chip); }
  .pill[data-status="unrecognized"] { color: var(--danger); }
  ul { margin: 0; padding-left: 18px; }
  .confirm { display: inline-flex; gap: 6px; align-items: center; white-space: nowrap; font-size: 13px; }
</style>
