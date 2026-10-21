<script lang="ts">
  // Comparaison de deux périodes : volumes, artistes gagnés et perdus, heures d'écoute, son.
  import { api, entityHref, type Comparison, type Period, type Side } from "$lib/api";
  import Chart from "$lib/Chart.svelte";
  import Cover from "$lib/Cover.svelte";
  import { axisStyle, base, type ThemeColors } from "$lib/theme";
  import { axes, esc, hours, num, errText } from "$lib/format";
  import { locale, t } from "$lib/settings.svelte";

  type Choice = { id: string; label: string; period: Period };

  const DAY = 86400;
  const now = Math.floor(Date.now() / 1000);
  let years = $state<number[]>([]);
  let pick = $state({ a: "prev12m", b: "12m" });
  /** Dates des périodes personnalisées (AAAA-MM-JJ). */
  let custom = $state({ a: { from: "", to: "" }, b: { from: "", to: "" } });
  let c = $state<Comparison | null>(null);
  let loading = $state(false);
  let error = $state("");

  const choices: Choice[] = $derived([
    { id: "12m", label: t("period.12m"), period: { from: now - 365 * DAY, to: null } },
    { id: "prev12m", label: t("cmp.prev12m"), period: { from: now - 730 * DAY, to: now - 365 * DAY - 1 } },
    ...years.map((y) => ({
      id: String(y),
      label: String(y),
      period: { from: new Date(y, 0, 1).getTime() / 1000, to: new Date(y + 1, 0, 1).getTime() / 1000 - 1 },
    })),
    { id: "custom", label: t("cmp.custom"), period: { from: null, to: null } },
  ]);

  const day = (s: string, end = false) => (s ? new Date(`${s}T00:00:00`).getTime() / 1000 + (end ? DAY - 1 : 0) : null);
  function periodOf(side: "a" | "b"): Period {
    if (pick[side] === "custom") return { from: day(custom[side].from), to: day(custom[side].to, true) };
    return choices.find((x) => x.id === pick[side])?.period ?? { from: null, to: null };
  }
  const labelOf = (side: "a" | "b") =>
    pick[side] === "custom" ? `${custom[side].from || "…"} → ${custom[side].to || "…"}` : (choices.find((x) => x.id === pick[side])?.label ?? "");

  $effect(() => {
    api.listSources().then((list) => {
      const ts = list.flatMap((s) => [s.first_ts, s.last_ts]).filter((x): x is number => x != null);
      if (!ts.length) return;
      const [a, b] = [new Date(Math.min(...ts) * 1000).getFullYear(), new Date(Math.max(...ts) * 1000).getFullYear()];
      years = Array.from({ length: b - a + 1 }, (_, i) => b - i);
    });
  });

  $effect(() => {
    const [a, b] = [periodOf("a"), periodOf("b")];
    let alive = true;
    loading = true;
    api
      .compare(a, b)
      .then((r) => alive && ((c = r), (error = "")))
      .catch((e) => alive && (error = errText(e)))
      .finally(() => alive && (loading = false));
    return () => (alive = false);
  });

  const pct = (x: number) => new Intl.NumberFormat(locale(), { maximumFractionDigits: 1 }).format(x);
  /** Variation de B par rapport à A, en %. */
  function delta(a: number, b: number) {
    if (!a) return "";
    const d = Math.round((100 * (b - a)) / a);
    return d > 0 ? `+${d} %` : `${d} %`;
  }

  function hourChart(col: ThemeColors) {
    if (!c) return {};
    const line = (s: Side, i: number, name: string) => ({
      type: "line" as const,
      name,
      data: s.hours.map((v) => Math.round(v * 10) / 10),
      smooth: true,
      symbol: "none",
      lineStyle: { color: col.series[i], width: 2 },
      itemStyle: { color: col.series[i] },
      areaStyle: { color: col.series[i], opacity: 0.08 },
    });
    return {
      ...base(col),
      grid: { left: 40, right: 12, top: 28, bottom: 24 },
      legend: { top: 0, textStyle: { color: col.fg2 } },
      tooltip: { ...base(col).tooltip, trigger: "axis" as const, valueFormatter: (v: number) => `${pct(v)} %` },
      xAxis: { type: "category" as const, data: Array.from({ length: 24 }, (_, h) => `${h} h`), ...axisStyle(col) },
      yAxis: { type: "value" as const, ...axisStyle(col), axisLabel: { color: col.fg3, formatter: (v: number) => `${v} %` } },
      series: [line(c.a, 0, labelOf("a")), line(c.b, 1, labelOf("b"))],
    };
  }

  function soundChart(col: ThemeColors) {
    if (!c?.a.sound || !c.b.sound) return {};
    const AX = axes();
    const item = (s: Side, i: number, name: string) => ({
      name,
      value: s.sound!.map((v) => v ?? 0),
      lineStyle: { color: col.series[i], width: 2, type: i ? ([6, 4] as number[]) : ("solid" as const) },
      itemStyle: { color: col.series[i] },
      areaStyle: { color: col.series[i], opacity: 0.1 },
    });
    return {
      ...base(col),
      legend: { top: 0, textStyle: { color: col.fg2 } },
      tooltip: {
        ...base(col).tooltip,
        trigger: "item" as const,
        formatter: (x: any) => `<b>${esc(x.name)}</b><br>` + AX.map((a, j) => `${a.label}${t("colon")}<b>${x.value[j].toFixed(2)}</b>`).join("<br>"),
      },
      radar: {
        radius: "64%",
        center: ["50%", "56%"],
        indicator: AX.map((a) => ({ name: a.label, min: 0, max: 1 })),
        axisName: { color: col.fg2, fontSize: 13 },
        splitLine: { lineStyle: { color: col.line } },
        axisLine: { lineStyle: { color: col.line } },
        splitArea: { show: false },
      },
      series: [{ type: "radar" as const, symbol: "circle", symbolSize: 5, data: [item(c.a, 0, labelOf("a")), item(c.b, 1, labelOf("b"))] }],
    };
  }
</script>

{#snippet picker(side: "a" | "b")}
  <div class="pick">
    <span class="dot" class:b={side === "b"}></span>
    <select bind:value={pick[side]} aria-label={t(side === "a" ? "cmp.a" : "cmp.b")}>
      {#each choices as x (x.id)}<option value={x.id}>{x.label}</option>{/each}
    </select>
    {#if pick[side] === "custom"}
      <input type="date" bind:value={custom[side].from} aria-label={t("kpi.from")} />
      <input type="date" bind:value={custom[side].to} aria-label={t("kpi.to")} />
    {/if}
  </div>
{/snippet}

{#snippet artists(s: Side, second: boolean)}
  <ol class="rank">
    {#each s.top_artists as [r, other], i (r.id)}
      {@const move = other == null ? null : other - (i + 1)}
      <li>
        <span class="mono muted">{i + 1}</span>
        <Cover kind="artist" id={r.id} name={r.name} size={28} />
        <a href={entityHref("artist", r.id)}>{r.name}</a>
        {#if !second}
          <span class="mono small move" title={t("cmp.rankThere")}>{other == null ? t("cmp.absent") : `→ ${other}`}</span>
        {:else}
          <span class="mono small move" class:up={move != null && move > 0} class:down={move != null && move < 0}>
            {#if other == null}{t("cmp.new")}{:else if move === 0}={:else}{move! > 0 ? "▲" : "▼"} {Math.abs(move!)}{/if}
          </span>
        {/if}
        <span class="mono muted small">{num(r.plays)}</span>
      </li>
    {/each}
  </ol>
{/snippet}

<div class="page">
  <header class="head">
    <div>
      <div class="eyebrow">{t("nav.compare")}</div>
      <h1>{t("cmp.title")}</h1>
    </div>
    <div class="pickers">
      {@render picker("a")}
      <span class="muted">{t("cmp.vs")}</span>
      {@render picker("b")}
    </div>
  </header>

  {#if error}
    <p class="panel" role="alert">{t("error", { msg: error })}</p>
  {:else if c}
    <section class="panel" class:dim={loading}>
      <table class="kpis-table">
        <thead>
          <tr><th></th><th><span class="dot"></span>{labelOf("a")}</th><th><span class="dot b"></span>{labelOf("b")}</th><th></th></tr>
        </thead>
        <tbody>
          <tr><th>{t("kpi.plays")}</th><td>{num(c.a.plays)}</td><td>{num(c.b.plays)}</td><td class="muted">{delta(c.a.plays, c.b.plays)}</td></tr>
          <tr><th>{t("kpi.listened")}</th><td>≈ {hours(c.a.listened_s)}</td><td>≈ {hours(c.b.listened_s)}</td><td class="muted">{delta(c.a.listened_s, c.b.listened_s)}</td></tr>
          <tr><th>{t("kinds.artist")}</th><td>{num(c.a.artists)}</td><td>{num(c.b.artists)}</td><td class="muted">{delta(c.a.artists, c.b.artists)}</td></tr>
          <tr><th>{t("cmp.newArtists")}</th><td>{num(c.a.new_artists)}</td><td>{num(c.b.new_artists)}</td><td class="muted">{delta(c.a.new_artists, c.b.new_artists)}</td></tr>
        </tbody>
      </table>
      <p class="muted small">{t("cmp.periodsNote")}</p>
    </section>

    <div class="two" class:dim={loading}>
      <section class="panel">
        <h2>{t("cmp.gained")}</h2>
        {#each [[c.gained, true], [c.lost, false]] as [list, up] (up)}
          {#if !up}<h2 class="sub">{t("cmp.lost")}</h2>{/if}
          {#if (list as Comparison["gained"]).length}
            <ol class="shift">
              {#each list as Comparison["gained"] as s (s.artist.id)}
                <li>
                  <Cover kind="artist" id={s.artist.id} name={s.artist.name} size={28} />
                  <a href={entityHref("artist", s.artist.id)}>{s.artist.name}</a>
                  <span class="mono small" class:up class:down={!up}>{pct(s.share_a)} % → {pct(s.share_b)} %</span>
                </li>
              {/each}
            </ol>
          {:else}
            <p class="muted">{t("disc.none")}</p>
          {/if}
        {/each}
        <p class="muted small">{t("cmp.shareNote")}</p>
      </section>
      <section class="panel">
        <h2>{t("cmp.when")}</h2>
        <Chart option={hourChart} height="260px" label={t("cmp.when")} />
        <h2 class="sub">{t("cmp.sound")}</h2>
        {#if c.a.sound && c.b.sound}
          <Chart option={soundChart} height="340px" label={t("cmp.sound")} />
        {:else}
          <p class="muted">{t("cmp.noSound")} <a href="/enrich">{t("feat.start")}</a>.</p>
        {/if}
      </section>
    </div>

    <div class="two" class:dim={loading}>
      <section class="panel">
        <h2><span class="dot"></span>{t("cmp.top", { period: labelOf("a") })}</h2>
        {@render artists(c.a, false)}
      </section>
      <section class="panel">
        <h2><span class="dot b"></span>{t("cmp.top", { period: labelOf("b") })}</h2>
        {@render artists(c.b, true)}
      </section>
    </div>
    <p class="muted small">{t("cmp.rankNote")}</p>
  {/if}
</div>

<style>
  .page { display: grid; gap: 20px; max-width: 1400px; }
  .head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: end; gap: 12px; }
  .pickers { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .pick { display: flex; gap: 6px; align-items: center; }
  select, input { font: inherit; color: var(--fg); background: var(--surface); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; }
  .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; background: var(--s1); margin-right: 6px; }
  .dot.b { background: var(--s2); }
  .two { display: grid; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr)); gap: 20px; }
  .dim { opacity: 0.55; transition: opacity 0.2s; }
  h2.sub { margin-top: 18px; }
  p { margin: 0; }
  .kpis-table { border-collapse: collapse; width: 100%; max-width: 720px; font-variant-numeric: tabular-nums; }
  .kpis-table th, .kpis-table td { text-align: left; padding: 6px 12px 6px 0; border-bottom: 1px solid var(--line); }
  .kpis-table thead th { font-weight: 500; color: var(--fg-2); }
  .kpis-table tbody th { font-weight: 400; color: var(--fg-2); }
  .kpis-table td { font-family: var(--font-mono); font-size: 15px; }
  @media (max-width: 900px) {
    :global(.android) .kpis-table td { white-space: nowrap; font-size: 14px; }
    :global(.android) .kpis-table th, :global(.android) .kpis-table td { padding-right: 8px; }
  }
  .rank, .shift { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .rank li { display: grid; grid-template-columns: 22px auto minmax(0, 1fr) 64px auto; gap: 10px; align-items: center; }
  .shift li { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 10px; align-items: center; }
  .rank a, .shift a { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .move { color: var(--fg-3); }
  .up { color: var(--s3); }
  .down { color: var(--danger); }
</style>
