<script lang="ts">
  import AboutPanel from "$lib/AboutPanel.svelte";
  import BackupPanel from "$lib/BackupPanel.svelte";
  import ScrobblerPanel from "$lib/ScrobblerPanel.svelte";
  import { android } from "$lib/platform";
  import {
    ACCENTS,
    DATE_FORMATS,
    LANGS,
    MIX_SIZES,
    settings,
    setAccent,
    setDateFormat,
    setLang,
    setMixSize,
    setTheme,
    t,
    type DateFormat,
    type LangChoice,
    type Theme,
  } from "$lib/settings.svelte";
  import { day } from "$lib/format";

  /** Exemple de chaque format, sur la date du jour. */
  function example(f: DateFormat): string {
    const s = day(new Date(), false, f);
    return f === "auto" ? t("set.dateFormat.auto", { example: s }) : s;
  }

  const themes: Theme[] = ["system", "light", "dark"];
</script>

<div class="page">
  <header>
    <h1>{t("set.title")}</h1>
  </header>

  {#if android}<ScrobblerPanel />{/if}

  <section class="panel">
    <div class="row">
      <label for="lang"><h2>{t("set.language")}</h2></label>
      <select id="lang" value={settings.langChoice} onchange={(e) => setLang(e.currentTarget.value as LangChoice)}>
        <option value="system">{t("set.language.system")}</option>
        {#each LANGS as l (l.id)}<option value={l.id} lang={l.id}>{l.name}</option>{/each}
      </select>
    </div>

    <div class="row">
      <label for="datefmt"><h2>{t("set.dateFormat")}</h2></label>
      <select id="datefmt" value={settings.dateFormat} onchange={(e) => setDateFormat(e.currentTarget.value as DateFormat)}>
        {#each DATE_FORMATS as f (f)}<option value={f}>{example(f)}</option>{/each}
      </select>
    </div>

    <div class="row">
      <h2 id="theme-label">{t("set.theme")}</h2>
      <div class="seg" role="group" aria-labelledby="theme-label">
        {#each themes as th (th)}
          <button aria-pressed={settings.theme === th} onclick={() => setTheme(th)}>{t(`set.theme.${th}`)}</button>
        {/each}
      </div>
    </div>
    <p class="muted small">{t("set.theme.help")}</p>

    <div class="row">
      <h2 id="accent-label">{t("set.accent")}</h2>
      <div class="swatches" role="radiogroup" aria-labelledby="accent-label">
        {#each ACCENTS as a (a)}
          <button
            class="swatch"
            data-swatch={a}
            role="radio"
            aria-checked={settings.accent === a}
            title={t(`set.accent.${a}`)}
            aria-label={t(`set.accent.${a}`)}
            onclick={() => setAccent(a)}
          ></button>
        {/each}
      </div>
    </div>
  </section>

  <section class="panel">
    <div class="row">
      <label for="mixsize"><h2>{t("set.mixSize")}</h2></label>
      <select id="mixsize" value={String(settings.mixSize)} onchange={(e) => setMixSize(Number(e.currentTarget.value))}>
        {#each MIX_SIZES as n (n)}<option value={n}>{t("set.mixSize.n", { n: Number(n), h: Math.round((Number(n) * 3.6) / 6) / 10 })}</option>{/each}
      </select>
    </div>
    <p class="muted small">{t("set.mixSize.help")}</p>
  </section>

  <BackupPanel />

  <AboutPanel />
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 720px; }
  .row { display: flex; flex-wrap: wrap; gap: 8px 16px; align-items: center; justify-content: space-between; }
  .row label { cursor: pointer; }
  select { font: inherit; color: var(--fg); background: var(--surface); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; min-width: 200px; }
  p { margin: 0; }
  .swatches { display: flex; gap: 10px; }
  .swatch { width: 28px; height: 28px; border-radius: 50%; border: 2px solid var(--surface); box-shadow: 0 0 0 1px var(--line); padding: 0; cursor: pointer; }
  /* Android : zone à toucher de 40 px (règle commune des boutons), rond gardé. */
  :global(.android) .swatch { width: 40px; height: 40px; }
  .swatch[aria-checked="true"] { box-shadow: 0 0 0 2px var(--fg); }
  /* Teinte de chaque pastille, identique en clair et en sombre pour rester reconnaissable. */
  .swatch[data-swatch="blue"] { background: #2a78d6; }
  .swatch[data-swatch="violet"] { background: #6d55e0; }
  .swatch[data-swatch="green"] { background: #1f9e6a; }
  .swatch[data-swatch="orange"] { background: #e0662a; }
  .swatch[data-swatch="rose"] { background: #d13f86; }
</style>
