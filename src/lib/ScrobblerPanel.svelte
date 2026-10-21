<script lang="ts">
  // Scrobbleur Android : accès aux notifications, applis suivies, journal du téléphone.
  import { api, type ScrobblerStatus } from "$lib/api";
  import ScrobbleDestinations from "$lib/ScrobbleDestinations.svelte";
  import { day } from "$lib/format";
  import { notifLabels } from "$lib/notifs.svelte";
  import { locale, t } from "$lib/settings.svelte";

  let st = $state<ScrobblerStatus | null>(null);

  const load = () => api.scrobblerStatus().then((s) => (st = s)).catch(() => {});

  $effect(() => {
    load();
    // Retour des réglages système (accès accordé, appli lancée entre-temps).
    const onShow = () => document.visibilityState === "visible" && load();
    document.addEventListener("visibilitychange", onShow);
    return () => document.removeEventListener("visibilitychange", onShow);
  });

  async function toggle(pkg: string, enabled: boolean) {
    await api.scrobblerSetApp(pkg, enabled);
    load();
  }

  async function setNotifs(active: boolean, jobs: boolean) {
    await api.setNotifs(active, jobs, notifLabels(), true).catch(() => {});
    load();
  }

  function when(uts: number): string {
    const d = new Date(uts * 1000);
    return `${day(d)} ${d.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" })}`;
  }
</script>

{#if st}
  <section class="panel">
    <h2>{t("scr.title")}</h2>
    <p class="muted">{t("scr.help")}</p>

    {#if st.access}
      <p class="ok">{t("scr.access.on")}</p>
    {:else}
      <p>{t("scr.access.off")}</p>
      <div class="actions">
        <button class="btn primary" onclick={() => api.scrobblerOpen("access")}>{t("scr.access.grant")}</button>
        <button class="btn" onclick={() => api.scrobblerOpen("app")}>{t("scr.appInfo")}</button>
      </div>
      <p class="muted small">{t("scr.access.restricted")}</p>
    {/if}

    <h3>{t("scr.apps")}</h3>
    {#if st.apps.length}
      <ul>
        {#each st.apps as a (a.pkg)}
          <li>
            <label>
              <input type="checkbox" checked={a.enabled} onchange={(e) => toggle(a.pkg, e.currentTarget.checked)} />
              <span class="name">{a.label}</span>
            </label>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">{t("scr.apps.none")}</p>
    {/if}

    <ScrobbleDestinations {st} reload={load} />

    <h3>{t("scr.notif")}</h3>
    <ul>
      <li>
        <label>
          <input type="checkbox" checked={st.notif.active} onchange={(e) => setNotifs(e.currentTarget.checked, st!.notif.jobs)} />
          <span>{t("scr.notif.active")}</span>
        </label>
      </li>
      <li>
        <label>
          <input type="checkbox" checked={st.notif.jobs} onchange={(e) => setNotifs(st!.notif.active, e.currentTarget.checked)} />
          <span>{t("scr.notif.jobs")}</span>
        </label>
      </li>
    </ul>
    {#if (st.notif.active || st.notif.jobs) && !st.notif.allowed}
      <p class="small">{t("scr.notif.denied")}</p>
      <div class="actions"><button class="btn" onclick={() => api.scrobblerOpen("app")}>{t("scr.appInfo")}</button></div>
    {/if}

    <p class="muted small">
      {t("scr.count", { n: st.count })}
      {#if st.last}<br />{t("scr.last", { track: st.last.track, artist: st.last.artist, when: when(st.last.uts) })}{/if}
    </p>
  </section>
{/if}

<style>
  p { margin: 0; }
  h3 { font-size: 15px; font-weight: 500; }
  .ok { color: var(--fg-2); }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; }
  li + li { border-top: 1px solid var(--line); }
  label { display: flex; align-items: center; gap: 12px; padding: 10px 2px; cursor: pointer; }
  input { width: 20px; height: 20px; accent-color: var(--accent); margin: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
