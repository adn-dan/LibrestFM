<script lang="ts">
  // Où vont les écoutes du téléphone : fichier local et/ou comptes (Libre.fm, Last.fm, ListenBrainz…).
  import { api, type NewDestination, type ScrobblerDestination, type ScrobblerStatus } from "$lib/api";
  import { errText } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  let { st, reload }: { st: ScrobblerStatus; reload: () => void } = $props();

  type Choice = "librefm" | "lastfm" | "listenbrainz" | "gnufm" | "lbcustom";
  const CHOICES: { id: Choice; label: () => string }[] = [
    { id: "librefm", label: () => "Libre.fm" },
    { id: "lastfm", label: () => "Last.fm" },
    { id: "listenbrainz", label: () => "ListenBrainz" },
    { id: "gnufm", label: () => t("scr.dest.gnufmCustom") },
    { id: "lbcustom", label: () => t("scr.dest.lbCustom") },
  ];

  let adding = $state(false);
  let choice = $state<Choice>("librefm");
  let url = $state("");
  let user = $state("");
  let secret = $state("");
  let apiKey = $state("");
  let apiSecret = $state("");
  let busy = $state(false);
  let error = $state("");

  const token = $derived(choice === "listenbrainz" || choice === "lbcustom");
  const custom = $derived(choice === "gnufm" || choice === "lbcustom");

  function name(d: ScrobblerDestination): string {
    if (d.kind === "lastfm") return "Last.fm";
    if (d.kind === "gnufm" && d.url.includes("libre.fm")) return "Libre.fm";
    if (d.kind === "listenbrainz" && d.url.includes("api.listenbrainz.org")) return "ListenBrainz";
    try {
      return new URL(d.url).host;
    } catch {
      return d.url;
    }
  }

  async function connect(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    const d: NewDestination = {
      kind: choice === "lastfm" ? "lastfm" : token ? "listenbrainz" : "gnufm",
      url: custom ? url : undefined,
      user,
      secret,
      apiKey: choice === "lastfm" ? apiKey : undefined,
      apiSecret: choice === "lastfm" ? apiSecret : undefined,
    };
    try {
      await api.scrobblerAddDestination(d);
      adding = false;
      user = secret = apiKey = apiSecret = url = "";
      reload();
    } catch (err) {
      error = errText(err).includes("auth") ? t("scr.dest.err.auth") : t("scr.dest.err.network");
    } finally {
      // Le mot de passe ne reste pas dans le formulaire.
      secret = "";
      busy = false;
    }
  }

  async function act(p: Promise<void>) {
    await p.catch(() => {});
    reload();
  }
</script>

<h3>{t("scr.dest")}</h3>
<ul>
  <li>
    <label>
      <input type="checkbox" checked={st.local} onchange={(e) => act(api.scrobblerSetLocal(e.currentTarget.checked))} />
      <span class="name">{t("scr.dest.local")}</span>
    </label>
    {#if !st.local}<p class="state small">{t("scr.dest.localOff")}</p>{/if}
  </li>
  {#each st.destinations as d (d.id)}
    <li>
      <label>
        <input type="checkbox" checked={d.enabled} onchange={(e) => act(api.scrobblerSetDestination(d.id, e.currentTarget.checked))} />
        <span class="name">{name(d)}{#if d.user}<span class="muted"> · {d.user}</span>{/if}</span>
      </label>
      <button class="link" onclick={() => act(api.scrobblerRemoveDestination(d.id))}>{t("scr.dest.remove")}</button>
      {#if d.pending > 0 || d.error}
        <p class="state small" class:bad={d.error === "auth"}>
          {#if d.error}{t(d.error === "auth" ? "scr.dest.err.auth" : "scr.dest.err.network")}{/if}
          {#if d.pending > 0}{t("scr.dest.pending", { n: d.pending })}{/if}
        </p>
      {/if}
    </li>
  {/each}
</ul>
{#if !st.local && !st.destinations.some((d) => d.enabled)}
  <p class="bad small">{t("scr.dest.none")}</p>
{/if}

{#if adding}
  <form onsubmit={connect}>
    <label class="field">
      <span>{t("scr.dest.kind")}</span>
      <select bind:value={choice}>
        {#each CHOICES as c (c.id)}<option value={c.id}>{c.label()}</option>{/each}
      </select>
    </label>
    {#if custom}
      <label class="field"><span>{t("scr.dest.url")}</span><input type="url" required bind:value={url} placeholder="https://" autocomplete="off" /></label>
    {/if}
    {#if token}
      <label class="field"><span>{t("scr.dest.token")}</span><input required bind:value={secret} autocomplete="off" /></label>
      <p class="muted small">{t("scr.dest.tokenHelp")}</p>
    {:else}
      <label class="field"><span>{t("scr.dest.user")}</span><input required bind:value={user} autocomplete="username" autocapitalize="none" /></label>
      <label class="field"><span>{t("scr.dest.password")}</span><input type="password" required bind:value={secret} autocomplete="current-password" /></label>
      <p class="muted small">{t("scr.dest.passwordHelp")}</p>
    {/if}
    {#if choice === "lastfm"}
      <label class="field"><span>{t("scr.dest.apiKey")}</span><input required bind:value={apiKey} autocomplete="off" /></label>
      <label class="field"><span>{t("scr.dest.apiSecret")}</span><input required bind:value={apiSecret} autocomplete="off" /></label>
      <p class="muted small">{t("scr.dest.apiHelp")}</p>
    {/if}
    {#if error}<p class="bad small">{error}</p>{/if}
    <div class="actions">
      <button class="btn primary" type="submit" disabled={busy}>{t("scr.dest.connect")}</button>
      <button class="btn" type="button" onclick={() => (adding = false)}>{t("scr.dest.cancel")}</button>
    </div>
  </form>
{:else}
  <div class="actions"><button class="btn" onclick={() => (adding = true)}>{t("scr.dest.add")}</button></div>
{/if}

<style>
  p { margin: 0; }
  h3 { font-size: 15px; font-weight: 500; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; }
  li { display: flex; flex-wrap: wrap; align-items: center; column-gap: 8px; }
  li + li { border-top: 1px solid var(--line); }
  li label { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; padding: 10px 2px; cursor: pointer; }
  input[type="checkbox"] { width: 20px; height: 20px; accent-color: var(--accent); margin: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { flex-basis: 100%; padding: 0 0 10px 34px; color: var(--fg-3); display: flex; flex-wrap: wrap; gap: 4px 10px; }
  .bad { color: var(--danger); }
  .link { border: 0; background: none; color: var(--fg-3); padding: 8px; font-size: 13px; }
  form { display: grid; gap: 10px; }
  .field { display: grid; gap: 4px; font-size: 13px; color: var(--fg-2); }
  .field input, .field select { font: inherit; font-size: 15px; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 7px; padding: 9px 10px; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; }
</style>
