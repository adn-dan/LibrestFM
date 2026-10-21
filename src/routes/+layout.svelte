<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { goto, pushState } from "$app/navigation";
  import { api, entityHref, type SearchResults, type Ranked } from "$lib/api";
  import { num } from "$lib/format";
  import Cover from "$lib/Cover.svelte";
  import DetailDrawer from "$lib/DetailDrawer.svelte";
  import { afterNavigate } from "$app/navigation";
  import { closeDrill } from "$lib/drill.svelte";

  /** Android : sur écran étroit, le menu devient un panneau qu'on ouvre (☰) ; le PC garde la barre latérale.
   *  Ouvert, il occupe une entrée d'historique : le retour d'Android le referme. */
  const menuOpen = $derived(!!page.state.menu);
  function toggleMenu() {
    if (menuOpen) history.back();
    else pushState("", { menu: true });
  }
  /** Lien suivi depuis le panneau : la page remplace l'entrée du panneau (retour : page précédente). */
  function fromMenu(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (!menuOpen || !a || a.origin !== location.origin || e.defaultPrevented) return;
    e.preventDefault();
    goto(a.pathname + a.search, { replaceState: true });
  }

  afterNavigate(() => closeDrill());

  // Android : barre d'état et barre de navigation aux couleurs du thème de l'app.
  $effect(() => {
    if (!android) return;
    const theme = settings.theme;
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const apply = () => api.setSystemBars(theme === "dark" || (theme === "system" && mq.matches)).catch(() => {});
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  });

  // Android : textes des notifications dans la langue de l'app, avancement des téléchargements.
  $effect(() => {
    if (!android) return;
    const labels = notifLabels();
    api.scrobblerStatus().then((s) => api.setNotifs(s.notif.active, s.notif.jobs, labels)).catch(() => {});
  });
  $effect(() => {
    if (!android) return;
    // Lu pour suivre chaque changement d'état des tâches.
    void [sync.st, features.st, credits.st, infos.st];
    pushProgress();
  });

  // Android : notification touchée (scrobbleur) : page « En écoute ».
  $effect(() => {
    if (!android) return;
    const check = () => {
      if (document.visibilityState !== "visible") return;
      api
        .takeOpen()
        .then((r) => {
          if (r.page === "now" && page.url.pathname !== "/now") goto("/now");
        })
        .catch(() => {});
    };
    check();
    document.addEventListener("visibilitychange", check);
    return () => document.removeEventListener("visibilitychange", check);
  });

  // Écoutes enregistrées par le scrobbleur du téléphone : lues au retour dans l'app et chaque minute.
  $effect(() => {
    if (!android) return;
    const scan = () => {
      if (document.visibilityState !== "visible") return;
      api.scrobblerScan().then((r) => {
        if (r.files.some((f) => f.added > 0)) {
          sync.done++;
          refreshJobs();
        }
      }).catch(() => {});
    };
    const timer = setInterval(scan, 60_000);
    document.addEventListener("visibilitychange", scan);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", scan);
    };
  });
  import EnrichBar from "$lib/EnrichBar.svelte";
  import { credits, features, infos, refreshJobs, watchJobs, type Job } from "$lib/enrich.svelte";
  import { android } from "$lib/platform";
  import { notifLabels, pushProgress } from "$lib/notifs.svelte";
  import { applySettings, settings, t } from "$lib/settings.svelte";
  import { sync, watchSync } from "$lib/sync.svelte";

  applySettings();

  $effect(() => watchJobs());
  $effect(() => watchSync());
  const jobs = [
    { job: features, label: "mini.features" as const, pending: "mini.pending" as const },
    { job: credits, label: "mini.credits" as const, pending: "mini.pending" as const },
    { job: infos, label: "mini.info" as const, pending: "mini.pendingItems" as const },
  ];
  function jobLabel(job: Job, pending: "mini.pending" | "mini.pendingItems"): string {
    const st = job.st;
    if (!st) return "";
    const wait = job.throttleLeft();
    if (st.running && wait > 0) return t("mini.throttled", { s: wait });
    if (st.running) return st.current ? t("mini.running", { name: st.current }) : t("mini.runningAnon");
    if (job.error) return t("mini.error");
    if (st.pending > 0) return t(pending, { n: st.pending });
    return t("mini.done");
  }

  let { children } = $props();

  const nav = $derived([
    { href: "/", label: t("nav.overview") },
    ...(android ? [{ href: "/now", label: t("nav.now") }] : []),
    { href: "/library", label: t("nav.library") },
    { href: "/compare", label: t("nav.compare") },
    { href: "/mixes", label: t("nav.mixes") },
    { href: "/discover", label: t("nav.discover") },
    { href: "/import", label: t("nav.sources") },
    { href: "/enrich", label: t("nav.features") },
    { href: "/cleanup", label: t("nav.cleanup") },
    { href: "/settings", label: t("nav.settings") },
  ]);

  let query = $state("");
  let results = $state<SearchResults | null>(null);
  let timer: ReturnType<typeof setTimeout>;
  /** Numéro de la dernière recherche : une réponse plus ancienne, arrivée après, est ignorée. */
  let searchId = 0;

  function onInput() {
    clearTimeout(timer);
    const id = ++searchId;
    const q = query.trim();
    if (q.length < 2) {
      results = null;
      return;
    }
    timer = setTimeout(async () => {
      const r = await api.search(q).catch(() => null);
      if (id === searchId) results = r;
    }, 200);
  }

  function open(r: Ranked) {
    searchId++;
    results = null;
    query = "";
    goto(entityHref(r.kind, r.id), { replaceState: menuOpen });
  }

  const groups = $derived(
    results
      ? ([
          [t("kinds.artist"), results.artists],
          [t("kinds.album"), results.albums],
          [t("kinds.track"), results.tracks],
        ] as [string, Ranked[]][]).filter(([, l]) => l.length)
      : [],
  );
</script>

<div class="shell" class:android>
  {#if android}
    <header class="topbar">
      <button class="burger" aria-label={t("nav.menu")} aria-expanded={menuOpen} onclick={toggleMenu}>
        <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M3 6h18M3 12h18M3 18h18" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
      </button>
      <div class="brand">Librest<span>FM</span></div>
    </header>
    <button class="scrim" class:open={menuOpen} tabindex="-1" aria-hidden="true" onclick={() => history.back()}></button>
  {/if}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
  <aside class:open={menuOpen} onclick={fromMenu}>
    <div class="brand">Librest<span>FM</span></div>
    <div class="search">
      <label class="sr" for="q">{t("search.label")}</label>
      <input
        id="q"
        type="search"
        placeholder={t("search.placeholder")}
        bind:value={query}
        oninput={onInput}
        onkeydown={(e) => e.key === "Escape" && (results = null)}
        autocomplete="off"
      />
      {#if results}
        <div class="results">
          {#each groups as [title, list] (title)}
            <div class="group">{title}</div>
            {#each list as r (r.kind + r.id)}
              <button class="hit" onclick={() => open(r)}>
                <span class="thumb"><Cover kind={r.kind} id={r.id} name={r.name} size={32} /></span>
                <span class="name">{r.name}</span>
                {#if r.artist}<span class="by">{r.artist}</span>{/if}
                <span class="plays mono">{num(r.plays)}</span>
              </button>
            {/each}
          {:else}
            <div class="none">{t("search.none")}</div>
          {/each}
        </div>
      {/if}
    </div>
    <div class="bottom">
      {#if sync.st?.running && sync.st.label}
        <a class="mini" href="/import">
          <span class="mini-state live">
            {sync.st.progress > 0 ? t("mini.sync", { label: sync.st.label, p: Math.round(sync.st.progress * 100) }) : t("mini.syncNew", { label: sync.st.label })}
          </span>
        </a>
      {/if}
      {#if features.st && features.st.ok + features.st.not_found + features.st.errors + features.st.pending > 0}
        <a class="mini" href="/enrich" aria-current={page.url.pathname === "/enrich" ? "page" : undefined}>
          {#each jobs as { job, label, pending } (label)}
            {#if job.st}
              <span class="mini-job">
                <span class="mini-head"><span>{t(label)}</span><span class="mono">{t("percent", { n: Math.round(job.st.plays_covered * 100) })}</span></span>
                <EnrichBar st={job.st} height={6} />
                <span class="mini-state" class:live={job.st.running}>{jobLabel(job, pending)}</span>
              </span>
            {/if}
          {/each}
        </a>
      {/if}
      <p class="credits">{t("credits.images")}<br />{t("credits.features")}<br />{t("credits.credits")}<br />{t("credits.lyrics")}</p>
    </div>
    <nav>
      {#each nav as n (n.href)}
        <a href={n.href} aria-current={page.url.pathname === n.href ? "page" : undefined}>{n.label}</a>
      {/each}
    </nav>
  </aside>
  <main>
    {@render children()}
  </main>
  <DetailDrawer />
</div>

<style>
  .shell { display: grid; grid-template-columns: 232px minmax(0, 1fr); height: 100%; }
  aside {
    border-right: 1px solid var(--line);
    background: var(--surface);
    padding: 20px 14px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    overflow-y: auto;
  }
  .brand { font-family: var(--font-display); font-weight: 700; font-size: 20px; letter-spacing: -0.01em; padding-left: 6px; }
  .brand span { color: var(--accent); }
  nav { display: grid; gap: 2px; }
  nav a { color: var(--fg-2); padding: 7px 10px; border-radius: 6px; }
  nav a:hover { background: var(--chip); text-decoration: none; }
  nav a[aria-current="page"] { background: var(--chip); color: var(--fg); font-weight: 500; }
  .search { position: relative; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
  input {
    width: 100%;
    font: inherit;
    color: var(--fg);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 7px;
    padding: 7px 10px;
  }
  .results {
    position: absolute;
    z-index: 20;
    top: calc(100% + 4px);
    left: 0;
    width: 360px;
    max-height: 70vh;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.14);
    padding: 6px;
  }
  .group { font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--fg-3); padding: 6px 8px 2px; }
  .hit {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    column-gap: 10px;
    width: 100%;
    text-align: left;
    background: none;
    border: 0;
    border-radius: 6px;
    padding: 5px 8px;
    color: var(--fg);
  }
  .hit:hover { background: var(--chip); }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thumb { grid-row: 1 / span 2; align-self: center; display: flex; }
  .name { grid-column: 2; }
  .by { grid-row: 2; grid-column: 2; font-size: 12.5px; color: var(--fg-3); }
  .plays { grid-row: 1 / span 2; grid-column: 3; align-self: center; font-size: 12.5px; color: var(--fg-3); }
  .none { padding: 10px; color: var(--fg-3); }
  .bottom { order: 99; margin-top: auto; display: grid; gap: 12px; }
  .mini-job { display: grid; gap: 6px; }
  .mini { display: grid; gap: 14px; padding: 10px; border-radius: 8px; color: var(--fg-2); font-size: 12.5px; }
  .mini:hover, .mini[aria-current="page"] { background: var(--chip); text-decoration: none; }
  .mini-head { display: flex; justify-content: space-between; color: var(--fg); font-weight: 500; }
  .mini-state { color: var(--fg-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mini-state.live::before { content: ""; display: inline-block; width: 6px; height: 6px; border-radius: 50%; background: var(--accent); margin-right: 6px; vertical-align: 1px; animation: pulse 1.6s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.3; } }
  .credits { margin: 0; padding-left: 6px; font-size: 11.5px; color: var(--fg-3); }
  main { overflow-y: auto; padding: 24px 28px 48px; }
  .topbar, .scrim { display: none; }
  @media (max-width: 900px) {
    .shell.android { grid-template-columns: minmax(0, 1fr); grid-template-rows: auto minmax(0, 1fr); }
    .android .topbar {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 6px 12px;
      background: var(--surface);
      border-bottom: 1px solid var(--line);
    }
    .burger { display: flex; padding: 8px; border: 0; border-radius: 8px; background: none; color: var(--fg); }
    .burger:active { background: var(--chip); }
    .android aside {
      position: fixed;
      inset: 0 auto 0 0;
      z-index: 30;
      width: min(300px, 85vw);
      transform: translateX(-100%);
      transition: transform 0.2s ease, visibility 0s linear 0.2s;
      box-shadow: 0 0 24px rgb(0 0 0 / 0.25);
      visibility: hidden;
    }
    .android aside.open { transform: none; visibility: visible; transition-delay: 0s; }
    .android .scrim {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 29;
      border: 0;
      background: rgb(0 0 0 / 0.4);
      opacity: 0;
      pointer-events: none;
      transition: opacity 0.2s ease;
    }
    .android .scrim.open { opacity: 1; pointer-events: auto; }
    .android .results { width: 100%; }
    .android nav a { padding: 10px 12px; }
    .android main { padding: 16px 14px 40px; }
  }
</style>
