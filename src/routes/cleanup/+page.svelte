<script lang="ts">
  import { api, entityHref, type MergeOp, type Suggestion } from "$lib/api";
  import { date, day, num } from "$lib/format";
  import { locale, t } from "$lib/settings.svelte";
  import type { Key } from "$lib/locales/fr";

  let suggestions = $state<Suggestion[] | null>(null);
  let ops = $state<MergeOp[]>([]);
  let dismissed = $state(0);
  let doubles = $state<[string, string, number][]>([]);
  let busy = $state(false);

  async function refresh() {
    [suggestions, ops, dismissed, doubles] = await Promise.all([api.suggestions(), api.mergeOps(), api.dismissedCount(), api.doubleSends()]);
  }

  $effect(() => {
    refresh();
  });

  /** Une action, puis tout est relu (une fusion peut en rendre d'autres caduques). */
  async function act(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await f();
      await refresh();
    } finally {
      busy = false;
    }
  }

  const RULES = ["html", "edition", "artist_in_title", "filename", "same_mbid", "multi_artist", "nickname", "featured", "album_variant", "article"] as const;
  const groups = $derived(
    RULES.map((rule) => ({ rule, list: (suggestions ?? []).filter((s) => s.rule === rule) })).filter((g) => g.list.length),
  );
  const ruleLabel = (r: string) => t(`cl.rule.${r}` as Key);
</script>

<div class="page">
  <header>
    <div class="eyebrow">{t("nav.cleanup")}</div>
    <h1>{t("cl.title")}</h1>
    <p class="intro">{t("cl.intro")}</p>
  </header>

  <section class="panel">
    <h2>{t("cl.suggestions")}</h2>
    {#if suggestions && !groups.length}
      <p class="muted">{t("cl.none")}</p>
    {/if}
    {#each groups as g (g.rule)}
      <div class="group">
        <div class="ghead">
          <div>
            <h3>{ruleLabel(g.rule)}</h3>
            <p class="muted small">{t(`cl.help.${g.rule}` as Key)}</p>
          </div>
          <button class="btn" disabled={busy} onclick={() => act(() => api.applySuggestions(g.list))}>
            {t("cl.applyAll", { n: g.list.length })}
          </button>
        </div>
        <ul>
          {#each g.list as s (s.key)}
            <li>
              <span class="from">
                <a href={entityHref(s.kind, s.from_id)}>{s.from_name}</a>
                {#if s.from_artist}<span class="muted small">{s.from_artist}</span>{/if}
              </span>
              <span class="arrow" aria-hidden="true">→</span>
              <span class="to">
                {#if s.to_id}<a href={entityHref(s.kind, s.to_id)}>{s.to_name}</a>{:else}<b>{s.to_name}</b> <span class="tag">{t("cl.new")}</span>{/if}
                {#if s.to_artist && s.to_artist !== s.from_artist}<span class="muted small">{s.to_artist}</span>{/if}
              </span>
              <span class="mono muted small">{t("plays", { n: s.plays })}</span>
              <span class="actions">
                <button class="btn" disabled={busy} onclick={() => act(() => api.applySuggestions([s]))}>{t("cl.apply")}</button>
                <button class="btn" disabled={busy} onclick={() => act(() => api.dismissSuggestions([s.key]))}>{t("cl.ignore")}</button>
              </span>
            </li>
          {/each}
        </ul>
      </div>
    {/each}
    {#if dismissed > 0}
      <p class="muted small">
        {t("cl.dismissed", { n: dismissed })}
        <button class="link" disabled={busy} onclick={() => act(() => api.clearDismissed())}>{t("cl.restore")}</button>
      </p>
    {/if}
  </section>

  <section class="panel">
    <h2>{t("cl.applied")}</h2>
    {#if ops.length}
      <ul class="ops">
        {#each ops as o (o.id)}
          <li>
            <span class="label">{o.label}</span>
            <span class="muted small">{ruleLabel(o.rule)} · {o.auto ? t("cl.auto") : t("cl.manual")} · {date(o.created_at)}</span>
            <button class="btn" disabled={busy} onclick={() => act(() => api.undoMerge(o.id))}>{t("cl.undo")}</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">{t("cl.appliedNone")}</p>
    {/if}
  </section>

  {#if doubles.length}
    <section class="panel">
      <div class="panel-head">
        <h2>{t("cl.doubles", { n: doubles.length })}</h2>
        <button class="btn" disabled={busy} onclick={() => act(() => api.restoreDoubleSends())}>{t("cl.doubles.restore")}</button>
      </div>
      <p class="muted small">{t("cl.doubles.help")}</p>
      <details>
        <summary class="small">{t("cl.doubles.show")}</summary>
        <ul class="ops">
          {#each doubles as [artist, title, ts], i (i)}
            <li><span class="label">{title} · {artist}</span><span class="muted small">{day(new Date(ts * 1000))} {new Date(ts * 1000).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" })}</span></li>
          {/each}
        </ul>
      </details>
    </section>
  {/if}
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 1100px; }
  header { display: grid; gap: 6px; }
  .intro { margin: 0; max-width: 75ch; color: var(--fg-2); }
  .group { display: grid; gap: 8px; padding-top: 12px; border-top: 1px solid var(--line); }
  .group:first-of-type { border-top: 0; padding-top: 0; }
  .ghead { display: flex; flex-wrap: wrap; gap: 8px 16px; justify-content: space-between; align-items: start; }
  h3 { font-family: var(--font-display); font-weight: 500; font-size: 15px; margin: 0; }
  p { margin: 0; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  li { display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) auto auto; gap: 4px 12px; align-items: center; padding: 4px 0; }
  .from, .to { display: grid; min-width: 0; }
  .from > *, .to > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .arrow { color: var(--fg-3); }
  .tag { font-size: 11px; background: var(--chip); border-radius: 999px; padding: 0 7px; color: var(--fg-2); }
  .actions { display: flex; gap: 6px; }
  .actions .btn, .ops .btn { padding: 4px 10px; font-size: 13px; }
  .ops li { grid-template-columns: minmax(0, 1fr) auto auto; }
  .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; font-size: inherit; }
  .link:hover { text-decoration: underline; }
  @media (max-width: 800px) {
    li { grid-template-columns: minmax(0, 1fr); }
    .arrow { display: none; }
  }
</style>
