<script lang="ts">
  // À propos : version, licence, code source, services de données et composants tiers.
  // La liste complète des composants (générée par `npm run licenses`) n'est chargée
  // qu'à l'ouverture.
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { num } from "$lib/format";
  import { t } from "$lib/settings.svelte";

  const SOURCE = "https://github.com/adn-dan/LibrestFM";
  const GPL = "https://www.gnu.org/licenses/gpl-3.0.html";

  /** Services interrogés, et ce que chacun fournit. */
  const SERVICES = [
    ["MusicBrainz", "https://musicbrainz.org", "ab.svc.musicbrainz"],
    ["ListenBrainz", "https://listenbrainz.org", "ab.svc.listenbrainz"],
    ["Deezer", "https://www.deezer.com", "ab.svc.deezer"],
    ["ReccoBeats", "https://reccobeats.com", "ab.svc.reccobeats"],
    ["LRCLIB", "https://lrclib.net", "ab.svc.lrclib"],
    ["Last.fm, Libre.fm", "https://www.last.fm", "ab.svc.scrobble"],
  ] as const;

  /** Composants principaux (la liste complète suit). */
  const MAIN = [
    ["Tauri", "MIT OR Apache-2.0"],
    ["Svelte, SvelteKit", "MIT"],
    ["Apache ECharts", "Apache-2.0"],
    ["SQLite, rusqlite", "Public domain, MIT"],
    ["IBM Plex Sans, IBM Plex Mono, Familjen Grotesk", "OFL-1.1"],
    ["Natural Earth (world-atlas)", "Public domain, ISC"],
    ["AndroidX, Material Components", "Apache-2.0"],
  ] as const;

  let version = $state("");
  let all = $state<{ packages: { n: string; v: string; l: string }[]; texts: Record<string, string> } | null>(null);

  $effect(() => {
    getVersion().then((v) => (version = v)).catch(() => {});
  });

  async function load(e: Event) {
    if (all || !(e.currentTarget as HTMLDetailsElement).open) return;
    all = (await import("$lib/third-party.json")).default;
  }

  /** Composants regroupés par licence. */
  const groups = $derived.by(() => {
    if (!all) return [];
    const m = new Map<string, string[]>();
    for (const p of all.packages) m.set(p.l || "?", [...(m.get(p.l || "?") ?? []), `${p.n} ${p.v}`]);
    return [...m.entries()].sort((a, b) => b[1].length - a[1].length);
  });

  /** Licences citées par une expression, sans doublon (« Apache-2.0 OR Apache-2.0 WITH … »). */
  const ids = (expr: string) => [...new Set(expr.split(/[()\s]+/).filter((x) => x && !["OR", "AND", "WITH"].includes(x)))];

  function open(e: MouseEvent) {
    e.preventDefault();
    openUrl((e.currentTarget as HTMLAnchorElement).href).catch(() => {});
  }
</script>

<section class="panel">
  <h2>{t("set.about")}</h2>
  <p class="about">{t("set.about.text")} {t("set.about.lyrics")}</p>
  <p>
    LibrestFM {version}. Copyright © 2026 adn-dan.
    {t("ab.license")} <a href={GPL} onclick={open}>GNU GPL v3</a>.
  </p>
  <p><a href={SOURCE} onclick={open}>{t("ab.source")} ↗</a></p>

  <h3>{t("ab.services")}</h3>
  <ul class="plain">
    {#each SERVICES as [name, url, key] (name)}
      <li><a href={url} onclick={open}>{name}</a> <span class="muted">· {t(key)}</span></li>
    {/each}
  </ul>

  <h3>{t("ab.components")}</h3>
  <ul class="plain">
    {#each MAIN as [name, license] (name)}
      <li>{name} <span class="muted mono small">· {license}</span></li>
    {/each}
  </ul>

  <details ontoggle={load}>
    <summary>{t("ab.all")}</summary>
    {#if all}
      <p class="muted small">{t("ab.count", { n: all.packages.length })}</p>
      {#each groups as [license, list] (license)}
        <details class="group">
          <summary><span class="mono">{license}</span> <span class="muted small">({num(list.length)})</span></summary>
          <p class="list small">{list.join(", ")}</p>
          {#each ids(license) as id (id)}
            {#if all.texts[id]}
              <details class="text">
                <summary class="small">{t("ab.text", { id })}</summary>
                <pre>{all.texts[id]}</pre>
              </details>
            {/if}
          {/each}
        </details>
      {/each}
    {/if}
  </details>
</section>

<style>
  p { margin: 0; }
  .about { max-width: 70ch; color: var(--fg-2); }
  h3 { font-size: 15px; font-weight: 500; margin: 4px 0 0; }
  .plain { margin: 0; padding-left: 18px; display: grid; gap: 4px; }
  details { display: grid; gap: 8px; }
  summary { cursor: pointer; }
  .group { margin-left: 12px; }
  .text { margin-left: 12px; }
  .list { color: var(--fg-2); overflow-wrap: anywhere; }
  pre { max-height: 320px; overflow: auto; white-space: pre-wrap; font-size: 12px; background: var(--chip); padding: 10px; border-radius: 6px; margin: 0; }
</style>
