<script lang="ts">
  import { num } from "$lib/format";
  import EnrichBar from "$lib/EnrichBar.svelte";
  import { anyPending, anyRunning, auto, credits, features, infos, setAuto, startAll, stopAll, type Job } from "$lib/enrich.svelte";
  import { t } from "$lib/settings.svelte";
  import type { Key } from "$lib/locales/fr";

  $effect(() => {
    features.refresh();
    credits.refresh();
    infos.refresh();
  });

  const sections: { job: Job; title: Key; intro: Key; note: Key; limits: Key[]; throttle?: Key }[] = [
    { job: features, title: "en.features.title", intro: "en.intro", note: "en.note", limits: ["en.limit1", "en.limit2", "en.limit3"], throttle: "en.throttle" },
    { job: credits, title: "en.credits.title", intro: "en.credits.intro", note: "en.credits.note", limits: ["en.credits.limit"] },
    { job: infos, title: "en.info.title", intro: "en.info.intro", note: "en.info.note", limits: ["en.info.limit"] },
  ];
</script>

<div class="page">
  <header class="head">
    <div>
      <div class="eyebrow">{t("nav.features")}</div>
      <h1>{t("en.title")}</h1>
    </div>
    {#if anyRunning()}
      <button class="btn" onclick={stopAll}>{t("en.stopAll")}</button>
    {:else}
      <button class="btn primary" onclick={startAll} disabled={!anyPending()}>{t("en.startAll")}</button>
    {/if}
  </header>
  <label class="auto">
    <input type="checkbox" checked={auto.on} onchange={(e) => setAuto(e.currentTarget.checked)} />
    <span>{t("en.auto")}<br /><span class="muted small">{t("en.auto.help")}</span></span>
  </label>

  {#each sections as s (s.title)}
    {@const st = s.job.st}
    {@const wait = s.job.throttleLeft()}
    <section class="panel">
      <h2>{t(s.title)}</h2>
      <p class="intro">{t(s.intro)}</p>

      {#if st}
        <EnrichBar {st} />
        <dl class="kpis">
          <div class="kpi"><dt><span class="dot ok"></span>{t("en.found")}</dt><dd>{num(st.ok)}</dd></div>
          <div class="kpi"><dt><span class="dot nf"></span>{t("en.notFound")}</dt><dd>{num(st.not_found)}</dd></div>
          <div class="kpi"><dt><span class="dot er"></span>{t("en.errors")}</dt><dd>{num(st.errors)}</dd></div>
          <div class="kpi"><dt><span class="dot"></span>{t("en.pending")}</dt><dd>{num(st.pending)}</dd></div>
          <div class="kpi"><dt>{t("en.covered")}</dt><dd>{t("percent", { n: Math.round(st.plays_covered * 100) })}</dd></div>
        </dl>

        {#if wait > 0 && s.throttle}
          <p class="throttle" role="status">
            {t(s.throttle)}
            <b>{st.running ? t("en.throttle.running", { s: wait }) : t("en.throttle.stopped", { s: wait })}</b>
          </p>
        {/if}

        <div class="actions">
          {#if st.running}
            <button class="btn" onclick={() => s.job.stop()}>{t("en.pause")}</button>
            <span class="muted current">{st.current ? t("en.running", { name: st.current }) : t("en.runningAnon")}</span>
          {:else}
            <button class="btn primary" onclick={() => s.job.start()} disabled={st.pending === 0 || wait > 0}>
              {st.ok + st.not_found + st.errors === 0 ? t("en.start") : t("en.resume")}
            </button>
            {#if st.errors + st.not_found > 0}
              <button class="btn" onclick={() => s.job.retry()} disabled={wait > 0}>{t("en.retry")}</button>
            {/if}
          {/if}
        </div>
      {/if}
      {#if s.job.error}<p role="alert" class="err">{t("en.stopped", { msg: s.job.error })}</p>{/if}
      <p class="muted small">{t(s.note)}</p>
      <details class="small muted">
        <summary>{t("en.limits")}</summary>
        <ul>
          {#each s.limits as l (l)}<li>{t(l)}</li>{/each}
        </ul>
      </details>
    </section>
  {/each}
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 900px; }
  .intro { margin: 0; max-width: 72ch; }
  .auto { display: flex; gap: 10px; align-items: flex-start; cursor: pointer; }
  .auto input { width: 18px; height: 18px; margin: 2px 0 0; accent-color: var(--accent); flex: none; }
  .head { display: flex; flex-wrap: wrap; gap: 12px; justify-content: space-between; align-items: end; }
  .dot { display: inline-block; width: 9px; height: 9px; border-radius: 2px; margin-right: 6px; background: var(--chip); border: 1px solid var(--line); }
  .dot.ok { background: var(--s3); border: 0; }
  .dot.nf { background: var(--s2); border: 0; }
  .dot.er { background: var(--danger); border: 0; }
  .actions { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; min-width: 0; }
  .current { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .err { color: var(--danger); margin: 0; }
  .throttle { margin: 0; padding: 10px 12px; border-radius: 8px; background: var(--chip); border-left: 3px solid var(--s2); }
  p { margin: 0; }
  summary { cursor: pointer; }
  ul { margin: 6px 0 0; padding-left: 18px; display: grid; gap: 4px; }
</style>
