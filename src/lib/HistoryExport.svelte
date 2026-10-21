<script lang="ts">
  // Export de l'historique : sources cochées, fusionnées, en CSV ou JSON ListenBrainz.
  import { shownPath, writeTo } from "$lib/androidFiles";
  import { errText, sourceName } from "./format";
  import { save } from "@tauri-apps/plugin-dialog";
  import { api, type SourceInfo } from "./api";
  import { t } from "./settings.svelte";
  import type { Key } from "./locales/fr";

  /** Change quand les sources changent (import, retrait, synchronisation). */
  let { version = 0 }: { version?: number } = $props();

  let sources = $state<SourceInfo[]>([]);
  let unchecked = $state<number[]>([]);
  let corrected = $state(true);
  let busy = $state(false);
  let message = $state("");

  $effect(() => {
    void version;
    api.listSources().then((s) => (sources = s));
  });

  const chosen = $derived(sources.filter((s) => !unchecked.includes(s.id)));

  function toggle(id: number) {
    unchecked = unchecked.includes(id) ? unchecked.filter((x) => x !== id) : [...unchecked, id];
  }

  async function run(format: "csv" | "listenbrainz") {
    const ext = format === "csv" ? "csv" : "jsonl";
    const path = await save({ defaultPath: `${t("ex.fileName")}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
    if (!path) return;
    busy = true;
    message = "";
    try {
      // Tout coché : liste vide, ce qui inclut aussi les sources ajoutées entre-temps.
      const ids = chosen.length === sources.length ? [] : chosen.map((s) => s.id);
      const name = `${t("ex.fileName")}.${ext}`;
      const n = await writeTo(path, name, (p) => api.exportHistory(ids, corrected, format, p));
      message = t("ex.done", { n, path: shownPath(path, name) });
    } catch (e) {
      message = t("error", { msg: errText(e) });
    } finally {
      busy = false;
    }
  }
</script>

{#if sources.length}
  <section class="panel">
    <h2>{t("ex.title")}</h2>
    <p class="muted small intro">{t("ex.intro")}</p>
    <div class="list">
      {#each sources as s (s.id)}
        <label>
          <input type="checkbox" checked={!unchecked.includes(s.id)} onchange={() => toggle(s.id)} />
          <span class="name">{sourceName(s.file_name)}</span>
          <span class="muted small">{t(`format.${s.format}` as Key)} · {t("acc.plays", { n: s.rows_added + s.rows_merged })}</span>
        </label>
      {/each}
    </div>
    {#if sources.length > 2}
      <p class="small">
        <button class="link" onclick={() => (unchecked = [])}>{t("ex.all")}</button> ·
        <button class="link" onclick={() => (unchecked = sources.map((s) => s.id))}>{t("ex.none")}</button>
      </p>
    {/if}
    <label class="opt"><input type="checkbox" bind:checked={corrected} /> {t("ex.corrected")}</label>
    <div class="actions">
      <button class="btn" disabled={busy || !chosen.length} onclick={() => run("csv")}>{t("ex.csv")}</button>
      <button class="btn" disabled={busy || !chosen.length} onclick={() => run("listenbrainz")}>{t("ex.lb")}</button>
    </div>
    {#if message}<p class="small" role="status">{message}</p>{/if}
  </section>
{/if}

<style>
  p { margin: 0; }
  .intro { max-width: 80ch; }
  .list { display: grid; gap: 4px; }
  label { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .name { font-weight: 500; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; font-size: inherit; }
  .link:hover { text-decoration: underline; }
</style>
