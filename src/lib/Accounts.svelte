<script lang="ts">
  // Comptes Libre.fm, Last.fm, ListenBrainz : ajout, état de la synchronisation, retrait.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, type Account, type Service } from "./api";
  import { date, num, errText } from "./format";
  import { clock } from "./enrich.svelte";
  import { sync, watchSync } from "./sync.svelte";
  import { t } from "./settings.svelte";

  /** Appelé quand les écoutes ont changé (sources à relire). */
  let { onchange }: { onchange?: () => void } = $props();

  const SERVICES: [Service, string][] = [
    ["librefm", "Libre.fm"],
    ["lastfm", "Last.fm"],
    ["listenbrainz", "ListenBrainz"],
  ];
  const label = (s: Service) => SERVICES.find((x) => x[0] === s)?.[1] ?? s;

  let accounts = $state<Account[]>([]);
  let service = $state<Service>("librefm");
  let username = $state("");
  let apiKey = $state("");
  let adding = $state(false);
  let error = $state("");
  let confirmRemove = $state<number | null>(null);

  const refresh = async () => (accounts = await api.accounts());

  $effect(() => {
    watchSync();
    refresh().catch((e) => (error = errText(e)));
  });

  // Fin d'une synchronisation : compteurs à jour.
  $effect(() => {
    if (sync.done) {
      refresh();
      onchange?.();
    }
  });

  async function add(e: Event) {
    e.preventDefault();
    if (adding || !username.trim()) return;
    adding = true;
    error = "";
    try {
      await api.addAccount(service, username.trim(), service === "lastfm" ? apiKey.trim() : null);
      username = "";
      await refresh();
    } catch (err) {
      error = String(err).startsWith("not_found\u001f") ? t("acc.notFound", { service: label(service) }) : t("acc.error", { msg: errText(err) });
    } finally {
      adding = false;
    }
  }

  async function remove(id: number) {
    confirmRemove = null;
    await api.removeAccount(id);
    await refresh();
    onchange?.();
  }

  const live = (a: Account) => (sync.st?.running && sync.st.account_id === a.id ? sync.st : null);
  const throttle = $derived(Math.max(0, Math.round((sync.st?.throttled_until ?? 0) - clock.now)));
</script>

<section class="panel">
  <div class="panel-head">
    <h2>{t("acc.title")}</h2>
    {#if accounts.length > 1}
      {#if sync.st?.running}
        <button class="btn" onclick={() => api.stopSync()}>{t("acc.stop")}</button>
      {:else}
        <button class="btn" onclick={() => api.startSync()}>{t("acc.syncAll")}</button>
      {/if}
    {/if}
  </div>
  <p class="muted small intro">{t("acc.intro")}</p>

  {#if accounts.length}
    <ul class="list">
      {#each accounts as a (a.id)}
        {@const st = live(a)}
        <li>
          <div class="who">
            <b>{a.username}</b>
            <span class="muted small">{label(a.service)}</span>
          </div>
          <div class="state small">
            <span>{t("acc.plays", { n: a.plays })}{#if a.plays}{" · "}<span class="muted">{t("acc.unique", { n: a.exclusive })}</span>{/if}</span>
            {#if st}
              <span class="live">
                {st.progress > 0 ? t("acc.running", { p: Math.round(st.progress * 100) }) : t("acc.runningNew")}
                <span class="muted">{t("acc.counts", { added: num(st.added), merged: num(st.merged) })}</span>
              </span>
              {#if st.progress > 0}<span class="bar"><span style:width={`${st.progress * 100}%`}></span></span>{/if}
              {#if throttle > 0}<span class="muted">{t("acc.throttled", { s: throttle })}</span>{/if}
            {:else if a.error}
              <span class="err">{t("acc.error", { msg: errText(a.error) })}</span>
            {:else if a.pending}
              <span class="muted">{t("acc.paused")}</span>
            {:else}
              <span class="muted">{a.synced_at ? t("acc.syncedAt", { date: date(a.synced_at) }) : t("acc.never")}</span>
            {/if}
          </div>
          <div class="actions">
            {#if confirmRemove === a.id}
              <span class="confirm">
                {t("acc.confirm", { n: a.exclusive })}
                <button class="btn danger" onclick={() => remove(a.id)}>{t("src.remove")}</button>
                <button class="btn" onclick={() => (confirmRemove = null)}>{t("src.cancel")}</button>
              </span>
            {:else}
              {#if st}
                <button class="btn" onclick={() => api.stopSync()}>{t("acc.stop")}</button>
              {:else}
                <button class="btn" disabled={sync.st?.running} onclick={() => api.startSync(a.id)}>{t("acc.sync")}</button>
              {/if}
              <button class="btn" onclick={() => (confirmRemove = a.id)}>{t("src.remove")}</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">{t("acc.none")}</p>
  {/if}

  <form class="add" onsubmit={add}>
    <div class="seg" role="group" aria-label={t("acc.title")}>
      {#each SERVICES as [code, name] (code)}
        <button type="button" aria-pressed={service === code} onclick={() => (service = code)}>{name}</button>
      {/each}
    </div>
    <label class="field">
      <span class="sr">{t("acc.user")}</span>
      <input bind:value={username} placeholder={t("acc.user")} autocomplete="off" spellcheck="false" />
    </label>
    {#if service === "lastfm"}
      <label class="field">
        <span class="sr">{t("acc.key")}</span>
        <input bind:value={apiKey} placeholder={t("acc.key")} autocomplete="off" spellcheck="false" class="mono" />
      </label>
    {/if}
    <button class="btn primary" disabled={adding || !username.trim() || (service === "lastfm" && !apiKey.trim())}>
      {adding ? t("acc.checking") : t("acc.add")}
    </button>
  </form>
  {#if service === "lastfm"}
    <p class="muted small">
      {t("acc.keyHelp")}
      <a
        href="https://www.last.fm/api/account/create"
        onclick={(e) => {
          e.preventDefault();
          openUrl(e.currentTarget.href).catch(() => {});
        }}>{t("acc.keyLink")}</a
      >. {t("acc.keyNote")}
    </p>
  {/if}
  {#if error}<p class="err small" role="alert">{error}</p>{/if}
</section>

<style>
  .intro { margin: 0; max-width: 80ch; }
  p { margin: 0; }
  .list { list-style: none; margin: 0; padding: 0; display: grid; }
  .list li {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr) auto;
    gap: 6px 16px;
    align-items: center;
    padding: 10px 0;
    border-bottom: 1px solid var(--line);
  }
  .who, .state { display: grid; gap: 2px; min-width: 0; }
  .live { color: var(--accent); }
  .bar { display: block; height: 4px; border-radius: 2px; background: var(--chip); overflow: hidden; max-width: 320px; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width 0.4s; }
  .err { color: var(--danger); }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; justify-content: end; }
  .actions .btn { padding: 4px 10px; font-size: 13px; }
  .confirm { display: inline-flex; gap: 6px; align-items: center; font-size: 13px; flex-wrap: wrap; }
  .add { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .field input {
    font: inherit;
    border: 1px solid var(--line);
    border-radius: 7px;
    padding: 6px 10px;
    background: var(--surface);
    color: var(--fg);
    width: 220px;
  }
  .field input.mono { width: 300px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
  @media (max-width: 800px) {
    .list li { grid-template-columns: minmax(0, 1fr); }
    .actions { justify-content: start; }
  }
</style>
