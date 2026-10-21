<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { api, entityHref, type PersonDetail } from "$lib/api";
  import Cover from "$lib/Cover.svelte";
  import { num, errText } from "$lib/format";
  import { roleFromPair, roleLabel } from "$lib/roles";
  import { t } from "$lib/settings.svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";

  let p = $state<PersonDetail | null>(null);
  let error = $state("");

  $effect(() => {
    const id = Number(page.url.searchParams.get("id"));
    p = null;
    error = "";
    api
      .person(id)
      // Aussi un artiste de l'historique : sa fiche artiste fait foi.
      .then((r) => {
        if (r.artist_id != null) goto(entityHref("artist", r.artist_id), { replaceState: true });
        else p = r;
      })
      .catch((e) => (error = errText(e)));
  });
</script>

{#if error}
  <p class="panel" role="alert">{t("error", { msg: error })}</p>
{:else if p}
  <div class="page">
    <header class="ident">
      <Cover kind="artist" id={p.id} name={p.name} size={96} remote={false} />
      <div class="titles">
        <div class="eyebrow">{t("person.eyebrow")}</div>
        <h1>{p.name}</h1>
        <a
          class="small"
          href={`https://musicbrainz.org/artist/${p.mbid}`}
          onclick={(e) => {
            // Ouvre le navigateur du système plutôt que la fenêtre de l'appli.
            e.preventDefault();
            openUrl(e.currentTarget.href).catch(() => {});
          }}>MusicBrainz ↗</a
        >
      </div>
    </header>

    <dl class="kpis">
      <div class="kpi"><dt>{t("kpi.plays")}</dt><dd>{num(p.total_plays)}</dd></div>
      <div class="kpi"><dt>{t("kinds.track")}</dt><dd>{num(p.tracks.length)}</dd></div>
    </dl>

    <section class="panel">
      <h2>{t("person.roles")}</h2>
      <div class="chips">
        {#each p.roles as [role, n] (role)}
          <span class="chip">{roleLabel(role)} <b class="mono">{num(n)}</b></span>
        {/each}
      </div>
    </section>

    <section class="panel">
      <h2>{t("person.tracks")}</h2>
      <div class="tablewrap">
        <table>
          <thead>
            <tr>
              <th>{t("person.col.track")}</th><th>{t("person.col.artist")}</th><th>{t("person.col.role")}</th>
              <th class="num">{t("kpi.plays")}</th>
            </tr>
          </thead>
          <tbody>
            {#each p.tracks as tr (tr.track_id)}
              <tr>
                <td><a href={entityHref("track", tr.track_id)}>{tr.title}</a></td>
                <td><a href={entityHref("artist", tr.artist_id)}>{tr.artist}</a></td>
                <td class="small">{tr.roles.map(roleFromPair).join(", ")}</td>
                <td class="num">{num(tr.plays)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="muted small">{t("person.note")}</p>
    </section>
  </div>
{/if}

<style>
  .page { display: grid; gap: 20px; max-width: 1100px; }
  .ident { display: flex; gap: 20px; align-items: flex-end; }
  .titles { display: grid; gap: 4px; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .chip { background: var(--chip); border-radius: 999px; padding: 4px 12px; font-size: 13px; }
  .chip b { font-weight: 500; color: var(--fg-2); margin-left: 4px; }
  p { margin: 0; }
</style>
