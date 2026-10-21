<script lang="ts">
  // Panneau latéral de détail : chiffres clés, tops et courbe d'une sélection.
  import { api, entityHref, type Detail, type Ranked } from "./api";
  import Cover from "./Cover.svelte";
  import TimeChart from "./TimeChart.svelte";
  import { date, num, errText } from "./format";
  import { closeDrill, drill } from "./drill.svelte";
  import { t } from "./settings.svelte";
  import { page } from "$app/state";
  import { goto, pushState } from "$app/navigation";
  import { android } from "./platform";

  // Android : le panneau ouvert occupe une entrée d'historique, le retour le referme.
  let pushed = false;
  $effect(() => {
    if (!android) return;
    if (drill.open && !pushed) {
      pushed = true;
      pushState("", { ...page.state, drill: true });
    } else if (!drill.open) {
      pushed = false;
    }
  });
  $effect(() => {
    // Retour d'Android : l'entrée du panneau a été quittée.
    if (android && pushed && !page.state.drill && drill.open) closeDrill();
  });
  function close() {
    if (android && page.state.drill) history.back();
    else closeDrill();
  }
  /** Lien suivi depuis le panneau : la page remplace l'entrée du panneau. */
  function follow(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (!android || !page.state.drill || !a || a.origin !== location.origin || e.defaultPrevented) return;
    e.preventDefault();
    goto(a.pathname + a.search, { replaceState: true });
  }

  let d = $state<Detail | null>(null);
  let error = $state("");

  $effect(() => {
    if (!drill.open) return;
    const filter = $state.snapshot(drill.filter);
    const top = drill.top;
    let alive = true;
    d = null;
    error = "";
    api
      .detail(filter, top)
      .then((r) => alive && (d = r))
      .catch((e) => alive && (error = errText(e)));
    return () => (alive = false);
  });

  const lists = $derived(
    d
      ? ([
          ["top.tracks", d.top_tracks],
          ["top.albums", d.top_albums],
          ["dr.topArtists", d.top_artists],
        ] as const)
      : [],
  );
</script>

<svelte:window onkeydown={(e) => drill.open && e.key === "Escape" && close()} />

{#if drill.open}
  <div class="backdrop" role="presentation" onclick={close}></div>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="drawer" role="dialog" aria-modal="true" aria-label={drill.title} tabindex="-1" onclick={follow}>
    <header>
      <div class="titles">
        <div class="eyebrow">{t("dr.eyebrow")}</div>
        <h2>{drill.title}</h2>
        {#if drill.subtitle}<p class="muted small">{drill.subtitle}</p>{/if}
      </div>
      <button class="btn" onclick={close}>{t("dr.close")}</button>
    </header>

    {#if error}
      <p role="alert">{t("error", { msg: error })}</p>
    {:else if !d}
      <p class="muted">…</p>
    {:else if d.total === 0}
      <p class="muted">{t("dr.empty")}</p>
    {:else}
      <dl class="kpis">
        <div class="kpi"><dt>{t("kpi.plays")}</dt><dd>{num(d.total)}</dd></div>
        <div class="kpi"><dt>{t("kinds.artist")}</dt><dd>{num(d.artists)}</dd></div>
        <div class="kpi"><dt>{t("kinds.album")}</dt><dd>{num(d.albums)}</dd></div>
        <div class="kpi"><dt>{t("kinds.track")}</dt><dd>{num(d.tracks)}</dd></div>
      </dl>
      {#if drill.timeline && d.per_month.length > 1}
        <p class="muted small">{date(d.first_ts)} → {date(d.last_ts)}</p>
        <TimeChart perMonth={d.per_month} perDay={d.per_day} height="170px" />
      {/if}
      {#each lists as [title, list] (title)}
        {#if list.length}
          <section>
            <h3>{t(title)}</h3>
            <ol>
              {#each list as Ranked[] as r (r.kind + r.id)}
                <li>
                  <Cover kind={r.kind} id={r.id} name={r.name} size={32} />
                  <span class="name">
                    <a href={entityHref(r.kind, r.id)} onclick={closeDrill}>{r.name}</a>
                    {#if r.artist}<span class="muted small">{r.artist}</span>{/if}
                  </span>
                  <span class="bar" aria-hidden="true"><i style:width={`${(100 * r.plays) / list[0].plays}%`}></i></span>
                  <span class="mono muted small">{num(r.plays)}</span>
                </li>
              {/each}
            </ol>
          </section>
        {/if}
      {/each}
    {/if}
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgb(0 0 0 / 0.25); z-index: 40; }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(560px, 94vw);
    z-index: 41;
    background: var(--bg);
    border-left: 1px solid var(--line);
    box-shadow: -12px 0 32px rgb(0 0 0 / 0.18);
    overflow-y: auto;
    padding: 20px 22px 40px;
    display: grid;
    gap: 16px;
    align-content: start;
    animation: slide 0.18s ease-out;
  }
  @keyframes slide { from { transform: translateX(24px); opacity: 0; } }
  /* Téléphone : plein écran, monte du bas ; l'en-tête reste visible. */
  @media (max-width: 900px) {
    :global(.android) .drawer { width: 100%; border-left: 0; box-shadow: none; padding: 0 14px 32px; animation: rise 0.2s ease-out; }
    :global(.android) .drawer header { position: sticky; top: 0; z-index: 1; background: var(--bg); padding: 14px 0 10px; border-bottom: 1px solid var(--line); }
    :global(.android) .drawer h2 { font-size: 20px; }
  }
  @keyframes rise { from { transform: translateY(32px); opacity: 0; } }
  header { display: flex; gap: 12px; justify-content: space-between; align-items: start; }
  .titles { display: grid; gap: 2px; min-width: 0; }
  h2 { font-size: 24px; font-weight: 700; }
  h3 { font-family: var(--font-display); font-weight: 500; font-size: 15px; margin: 0 0 8px; }
  p { margin: 0; }
  ol { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  li { display: grid; grid-template-columns: auto minmax(0, 1fr) 70px 4ch; gap: 10px; align-items: center; }
  .name { display: grid; min-width: 0; }
  .name > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bar { height: 6px; border-radius: 3px; background: var(--chip); position: relative; overflow: hidden; }
  .bar i { position: absolute; inset: 0 auto 0 0; background: var(--s1); border-radius: 3px; }
  .mono { text-align: right; }
</style>
