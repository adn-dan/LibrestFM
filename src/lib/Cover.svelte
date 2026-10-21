<script lang="ts">
  // Photo d'artiste ou pochette (pour un titre : celle de son album).
  // Initiales en attendant, ou si Deezer n'a pas d'image.
  import { api, type Kind } from "./api";
  import { t } from "./settings.svelte";

  /** remote = false : pas d'image à chercher (ex. une personne créditée), initiales seulement.
   *  cached : image déjà connue (null : aucune), fournie par la liste ; rien n'est alors cherché. */
  let {
    kind,
    id,
    name,
    size = 40,
    remote = true,
    cached,
  }: { kind: Kind; id: number; name: string; size?: number; remote?: boolean; cached?: string | null } = $props();

  let src = $state<string | null>(null);

  $effect(() => {
    let alive = true;
    src = cached ?? null;
    if (!remote || cached !== undefined) return;
    api
      .image(kind, id)
      .then((s) => alive && (src = s))
      .catch(() => {});
    return () => (alive = false);
  });

  const initials = $derived(
    name
      .split(/\s+/)
      .filter((w) => /\p{L}|\p{N}/u.test(w))
      .slice(0, 2)
      .map((w) => [...w.replace(/^[^\p{L}\p{N}]+/u, "")][0] ?? "")
      .join("")
      .toUpperCase(),
  );
</script>

<span class="cover" style:--size={`${size}px`} aria-hidden={!src}>
  {#if src}
    <img {src} alt={t(kind === "artist" ? "cover.artist" : "cover.album", { name })} />
  {:else}
    <span class="ph" style:font-size={`${Math.max(10, size * 0.34)}px`}>{initials}</span>
  {/if}
</span>

<style>
  .cover {
    flex: none;
    width: var(--size);
    height: var(--size);
    border-radius: max(3px, calc(var(--size) * 0.06));
    overflow: hidden;
    background: var(--chip);
    display: inline-grid;
    place-items: center;
  }
  img { width: 100%; height: 100%; object-fit: cover; display: block; animation: in 0.25s ease-out; }
  .ph { color: var(--fg-3); font-family: var(--font-display); font-weight: 500; letter-spacing: 0.02em; }
  @keyframes in { from { opacity: 0; } }
</style>
