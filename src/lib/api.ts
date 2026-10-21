// Appels au cœur Rust. Les types reflètent les structures sérialisées de librestfm-core.
import { invoke } from "@tauri-apps/api/core";

export type Kind = "artist" | "album" | "track";

export interface Period {
  from: number | null;
  to: number | null;
}

export interface Ranked {
  kind: Kind;
  id: number;
  name: string;
  artist: string | null;
  artist_id: number | null;
  plays: number;
  /** Temps d'écoute estimé (s), quand le classement est fait au temps. */
  seconds?: number;
}

export interface Overview {
  total: number;
  artists: number;
  albums: number;
  tracks: number;
  first_ts: number | null;
  last_ts: number | null;
  listened_s: number;
  listened_min_s: number;
  listened_known: number;
  per_month: [string, number][];
  per_day: [string, number][];
  top_artists: Ranked[];
  top_albums: Ranked[];
  top_tracks: Ranked[];
  heatmap: [number, number, number][];
  /** [année de sortie, écoutes] */
  release_years: [number, number][];
  release_covered: number;
  /** [code pays ISO, écoutes, artistes] */
  countries: [string, number, number][];
  countries_covered: number;
  /** [langue ISO 639-3, écoutes] */
  languages: [string, number][];
  languages_covered: number;
}

export interface Side {
  plays: number;
  listened_s: number;
  artists: number;
  new_artists: number;
  /** Artiste et son rang dans l'autre période (null : absent). */
  top_artists: [Ranked, number | null][];
  hours: number[];
  sound: (number | null)[] | null;
  sound_basis: number;
}

export interface Comparison {
  a: Side;
  b: Side;
  gained: { artist: Ranked; plays_a: number; share_a: number; share_b: number }[];
  lost: { artist: Ranked; plays_a: number; share_a: number; share_b: number }[];
}

export interface BackupInfo {
  plays: number;
  first_ts: number | null;
  last_ts: number | null;
  bytes: number;
}

export type BrowseSort = "plays" | "time" | "first" | "last" | "name";

export interface BrowsePage {
  total: number;
  rows: { item: Ranked; seconds: number; first_ts: number; last_ts: number }[];
}

/** Paroles (LRCLIB) : synchronisées (lines) en priorité, sinon texte brut. */
export interface Lyrics {
  lines: { ms: number; text: string }[];
  plain: string | null;
  instrumental: boolean;
}

/** Android : titre en cours dans une autre appli. */
export interface NowPlaying {
  access: boolean;
  track?: { artist: string; title: string; album: string | null; albumArtist: string | null; durationMs: number; positionMs: number; playing: boolean; player: string };
}

/** Scrobbleur Android : lecteur proposé, suivi ou non. */
export interface ScrobblerApp {
  pkg: string;
  label: string;
  enabled: boolean;
  /** Déjà vu en train de jouer quelque chose. */
  seen: boolean;
}

/** Compte vers lequel les écoutes du téléphone sont envoyées. */
export interface ScrobblerDestination {
  id: string;
  kind: "lastfm" | "gnufm" | "listenbrainz";
  url: string;
  user: string;
  enabled: boolean;
  /** Dernier envoi : identifiants refusés ou réseau/serveur indisponible. */
  error: "auth" | "network" | null;
  pending: number;
}

export interface NewDestination {
  kind: "lastfm" | "gnufm" | "listenbrainz";
  url?: string;
  user?: string;
  /** Mot de passe (utilisé une fois) ou jeton ListenBrainz. */
  secret: string;
  apiKey?: string;
  apiSecret?: string;
}

export interface ScrobblerStatus {
  /** Accès aux notifications accordé (sans lui, rien n'est détecté). */
  access: boolean;
  apps: ScrobblerApp[];
  /** Écoutes dans le journal du téléphone. */
  count: number;
  /** Enregistrement dans le fichier local. */
  local: boolean;
  destinations: ScrobblerDestination[];
  /** Notifications : scrobbleur actif, téléchargements ; allowed : autorisées par le système. */
  notif: { active: boolean; jobs: boolean; allowed: boolean };
  last?: { artist: string; track: string; uts: number };
}

export interface InboxReport {
  dir: string;
  files: ImportReport[];
  unchanged: number;
  cleaned: number;
}

export interface Turn {
  artist: Ranked;
  first_ts: number;
  last_ts: number;
  /** Écoutes des 3 mois après la découverte, ou des 6 mois avant l'abandon. */
  burst: number;
}

export interface Discoveries {
  per_month: [string, number][];
  found: Turn[];
  dropped: Turn[];
}

export interface SessionStats {
  count: number;
  avg_tracks: number;
  median_minutes: number;
  longest: { start_ts: number; tracks: number; minutes: number } | null;
  /** [tranche « 1 », « 2-3 »… « 21+ », sessions] */
  lengths: [string, number][];
  start_hours: number[];
  openers: Ranked[];
  closers: Ranked[];
}

export interface AlbumInfo {
  status: string;
  /** Première sortie : « 2004-02-26 », « 2004-02 » ou « 2004 ». */
  release_date: string | null;
  edition_date: string | null;
  duration_s: number | null;
  nb_tracks: number | null;
  label: string | null;
  record_type: string | null;
}

export interface ArtistInfo {
  status: string;
  /** Person, Group, Orchestra, Choir… */
  kind: string | null;
  country: string | null;
  area: string | null;
  begin: string | null;
  end: string | null;
}

export interface Features {
  status: string;
  provider: string;
  matched_title: string | null;
  matched_artist: string | null;
  isrc: string | null;
  axes: (number | null)[];
  tempo: number | null;
  loudness: number | null;
  key: number | null;
  mode: number | null;
}

export interface Profile {
  /** self : l'entité elle-même ; album / artist : moyenne ; library : toute la bibliothèque. */
  kind: "self" | "album" | "artist" | "library";
  name: string | null;
  basis: number;
  axes: (number | null)[];
  percentiles: (number | null)[];
}

export interface EntityDetail {
  kind: Kind;
  id: number;
  name: string;
  artist: string | null;
  artist_id: number | null;
  total: number;
  listened_s: number;
  first_ts: number | null;
  last_ts: number | null;
  per_month: [string, number][];
  per_day: [string, number][];
  top_tracks: Ranked[];
  top_albums: Ranked[];
  before: Ranked[];
  after: Ranked[];
  features: Features | null;
  duration_ms: number | null;
  album_info: AlbumInfo | null;
  artist_info: ArtistInfo | null;
  languages: string[];
  versions: Ranked[];
  /** Titre : artistes crédités dans l'ordre MusicBrainz (vide si un seul). */
  artist_credit: CreditedArtist[];
  merged: MergeOp[];
  profiles: Profile[];
}

export interface RejectCount {
  reason: string;
  count: number;
  sample_lines: number[];
}

export interface ImportReport {
  file_name: string;
  status: "imported" | "already_imported" | "unrecognized";
  source_id: number | null;
  format: string | null;
  rows_read: number;
  added: number;
  merged: number;
  rejected: number;
  rejects: RejectCount[];
  first_ts: number | null;
  last_ts: number | null;
  overlaps: { source_id: number; file_name: string; shared: number }[];
}

export interface SourceInfo {
  id: number;
  file_name: string;
  format: string;
  imported_at: number;
  rows_read: number;
  rows_added: number;
  rows_merged: number;
  rows_rejected: number;
  first_ts: number | null;
  last_ts: number | null;
  exclusive: number;
}

export interface SearchResults {
  artists: Ranked[];
  albums: Ranked[];
  tracks: Ranked[];
}

export interface EnrichStatus {
  ok: number;
  not_found: number;
  errors: number;
  pending: number;
  plays_covered: number;
  current_artist: string | null;
  running: boolean;
  /** Pause imposée par ReccoBeats : heure Unix de reprise. */
  throttled_until: number | null;
}

export interface TrackCredit {
  person_id: number;
  /** La personne est aussi un artiste de l'historique : sa fiche artiste la remplace. */
  artist_id: number | null;
  name: string;
  /** Type de relation MusicBrainz : writer, composer, producer, instrument, vocal… ou « credited ». */
  role: string;
  /** Instrument, type de voix… (en anglais, tel que MusicBrainz l'enregistre). */
  detail: string;
  tracks_in_library: number;
}

/** Artiste crédité d'un titre (ordre MusicBrainz), avec la liaison vers le suivant. */
export interface CreditedArtist {
  person_id: number;
  artist_id: number | null;
  name: string;
  /** « feat. », « & »… ; vide pour le dernier. */
  join: string;
}

export interface TrackCredits {
  status: "none" | "ok" | "not_found" | "error";
  recording_mbid: string | null;
  /** Code ISO 639-3 de la langue des paroles (zxx : sans paroles). */
  work_language: string | null;
  credits: TrackCredit[];
  /** Plusieurs artistes crédités, dans l'ordre ; vide sinon. */
  artist_credit: CreditedArtist[];
}

/** Filtre du panneau de détail : toutes les conditions présentes s'appliquent ensemble. */
export interface Filter {
  from?: number | null;
  to?: number | null;
  kind?: Kind | null;
  id?: number | null;
  country?: string | null;
  language?: string | null;
  release_from?: number | null;
  release_to?: number | null;
  dow?: number | null;
  hour?: number | null;
}

export interface Detail {
  total: number;
  artists: number;
  albums: number;
  tracks: number;
  first_ts: number | null;
  last_ts: number | null;
  top_artists: Ranked[];
  top_albums: Ranked[];
  top_tracks: Ranked[];
  per_month: [string, number][];
  per_day: [string, number][];
}

export interface Suggestion {
  key: string;
  rule: "edition" | "album_variant" | "filename" | "artist_in_title" | "same_mbid" | "article" | "html" | "multi_artist" | "nickname" | "featured";
  kind: Kind;
  from_id: number;
  from_name: string;
  from_artist: string | null;
  /** null : la cible sera créée. */
  to_id: number | null;
  to_name: string;
  to_artist: string | null;
  plays: number;
  sure: boolean;
}

export interface MergeOp {
  id: number;
  kind: Kind;
  rule: string;
  auto: boolean;
  label: string;
  created_at: number;
}

export interface AlbumCredits {
  tracks_total: number;
  tracks_done: number;
  /** Titres pas encore cherchés, les plus écoutés d'abord. */
  pending: number[];
  credits: (TrackCredit & { tracks: number })[];
}

export interface ArtistLink {
  artist: Ranked;
  people: { id: number; artist_id: number | null; name: string; roles: string[] }[];
}

export interface CreditedFor {
  artist: Ranked;
  roles: string[];
  /** [titre, nom, écoutes] */
  tracks: [number, string, number][];
}

export interface PersonDetail {
  id: number;
  artist_id: number | null;
  mbid: string;
  name: string;
  total_plays: number;
  roles: [string, number][];
  tracks: { track_id: number; title: string; artist: string; artist_id: number; plays: number; roles: string[] }[];
}

export interface CreditsStatus {
  ok: number;
  not_found: number;
  errors: number;
  pending: number;
  /** Crédits : fiches anciennes à relire (ordre des artistes). */
  refresh: number;
  plays_covered: number;
  current: string | null;
  running: boolean;
}

export const api = {
  /** Images déjà en cache (aucun téléchargement) : id → data URL. */
  cachedImages: (kind: Kind, ids: number[]) => invoke<Record<number, string>>("cached_images", { kind, ids }),
  browse: (kind: Kind, sort: BrowseSort, desc: boolean, filter: string, offset: number, limit = 100) =>
    invoke<BrowsePage>("browse", { kind, sort, desc, filter, offset, limit }),
  inputDir: () => invoke<string>("input_dir"),
  backupDb: (path: string) => invoke<BackupInfo>("backup_db", { path }),
  checkBackup: (path: string) => invoke<BackupInfo>("check_backup", { path }),
  restoreDb: (path: string) => invoke<BackupInfo>("restore_db", { path }),
  backups: () => invoke<{ dir: string; auto: [string, number][] }>("backups"),
  setInputDir: (path: string) => invoke<InboxReport>("set_input_dir", { path }),
  scanInput: () => invoke<InboxReport>("scan_input"),
  scrobblerStatus: () => invoke<ScrobblerStatus>("scrobbler_status"),
  scrobblerSetApp: (pkg: string, enabled: boolean) => invoke<void>("scrobbler_set_app", { pkg, enabled }),
  scrobblerOpen: (what: "access" | "app") => invoke<void>("scrobbler_open", { what }),
  scrobblerScan: () => invoke<InboxReport>("scrobbler_scan"),
  lyrics: (artist: string, title: string, album: string | null, durationMs: number | null, also: string[] = []) =>
    invoke<Lyrics>("lyrics", { artist, title, album, durationMs, also }),
  findTrack: (artist: string, title: string, albumArtist: string | null) =>
    invoke<{ id: number; title: string; artist: string } | null>("find_track", { artist, title, albumArtist }),
  nowPlaying: () => invoke<NowPlaying>("scrobbler_call", { cmd: "nowPlaying", args: {} }),
  mediaControl: (action: "play" | "pause" | "next" | "previous") => invoke<void>("scrobbler_call", { cmd: "mediaControl", args: { action } }),
  takeOpen: () => invoke<{ page: string | null }>("scrobbler_call", { cmd: "takeOpen", args: {} }),
  setSystemBars: (dark: boolean) => invoke<void>("scrobbler_call", { cmd: "setBars", args: { enabled: dark } }),
  keepAwake: (on: boolean) => invoke<void>("scrobbler_call", { cmd: "keepAwake", args: { enabled: on } }),
  setNotifs: (active: boolean, jobs: boolean, labels: Record<string, string>, ask = false) =>
    invoke<void>("scrobbler_call", { cmd: "setNotifs", args: { active, jobs, labels, ask } }),
  notifProgress: (text: string | null, percent: number | null) => invoke<void>("scrobbler_call", { cmd: "progress", args: { text, percent } }),
  scrobblerSetLocal: (enabled: boolean) => invoke<void>("scrobbler_call", { cmd: "setLocal", args: { enabled } }),
  scrobblerAddDestination: (d: NewDestination) => invoke<void>("scrobbler_call", { cmd: "addDestination", args: d }),
  scrobblerSetDestination: (id: string, enabled: boolean) => invoke<void>("scrobbler_call", { cmd: "setDestination", args: { id, enabled } }),
  scrobblerRemoveDestination: (id: string) => invoke<void>("scrobbler_call", { cmd: "removeDestination", args: { id } }),
  importFiles: (paths: string[]) => invoke<{ reports: ImportReport[]; cleaned: number }>("import_files", { paths }),
  suggestions: () => invoke<Suggestion[]>("suggestions"),
  applySuggestions: (list: Suggestion[]) => invoke<number>("apply_suggestions", { list }),
  dismissSuggestions: (keys: string[]) => invoke<void>("dismiss_suggestions", { keys }),
  clearDismissed: () => invoke<number>("clear_dismissed"),
  dismissedCount: () => invoke<number>("dismissed_count"),
  mergeEntities: (kind: Kind, from: number, to: number) => invoke<number>("merge_entities", { kind, from, to }),
  undoMerge: (op: number) => invoke<void>("undo_merge", { op }),
  renameEntity: (kind: Kind, id: number, name: string) => invoke<number>("rename_entity", { kind, id, name }),
  mergeOps: () => invoke<MergeOp[]>("merge_ops"),
  doubleSends: () => invoke<[string, string, number][]>("double_sends"),
  restoreDoubleSends: () => invoke<number>("restore_double_sends"),
  listSources: () => invoke<SourceInfo[]>("list_sources"),
  removeSource: (id: number) => invoke<number>("remove_source", { id }),
  overview: (period: Period, byTime = false, top = 15) => invoke<Overview>("overview", { period, top, byTime }),
  entity: (kind: Kind, id: number) => invoke<EntityDetail>("entity", { kind, id }),
  search: (query: string) => invoke<SearchResults>("search", { query }),
  /** Image d'une entité (data URL), téléchargée au besoin ; null si Deezer n'en a pas. */
  image: (kind: Kind, id: number) => invoke<string | null>("image", { kind, id }),
  /** Crédits d'un titre, récupérés sur MusicBrainz au premier affichage. */
  trackCredits: (trackId: number) => invoke<TrackCredits>("track_credits", { trackId }),
  albumCredits: (albumId: number) => invoke<AlbumCredits>("album_credits", { albumId }),
  person: (id: number) => invoke<PersonDetail>("person", { id }),
  artistLinks: (id: number) => invoke<ArtistLink[]>("artist_links", { id }),
  artistCredits: (id: number) => invoke<CreditedFor[]>("artist_credits", { id }),
  sessions: (period: Period) => invoke<SessionStats>("sessions", { period }),
  compare: (a: Period, b: Period) => invoke<Comparison>("compare", { a, b }),
  discoveries: (period: Period) => invoke<Discoveries>("discoveries", { period }),
  detail: (filter: Filter, top = 10) => invoke<Detail>("detail", { filter, top }),
  albumsByRelease: (fromYear: number, toYear: number, period: Period) =>
    invoke<{ album: Ranked; release_date: string }[]>("albums_by_release", { fromYear, toYear, period }),
  /** Infos d'album / d'artiste récupérées si jamais cherchées ; true si la fiche est à recharger. */
  ensureInfo: (kind: Kind, id: number) => invoke<boolean>("ensure_info", { kind, id }),
  infoStatus: () => invoke<CreditsStatus>("info_status"),
  startInfo: () => invoke<void>("start_info"),
  stopInfo: () => invoke<void>("stop_info"),
  retryInfoFailures: () => invoke<number>("retry_info_failures"),
  creditsStatus: () => invoke<CreditsStatus>("credits_status"),
  startCredits: () => invoke<void>("start_credits"),
  stopCredits: () => invoke<void>("stop_credits"),
  retryCreditFailures: () => invoke<number>("retry_credit_failures"),
  enrichStatus: () => invoke<EnrichStatus>("enrich_status"),
  startEnrich: () => invoke<void>("start_enrich"),
  stopEnrich: () => invoke<void>("stop_enrich"),
  retryEnrichFailures: () => invoke<number>("retry_enrich_failures"),
  accounts: () => invoke<Account[]>("accounts"),
  /** Vérifie le compte auprès du service puis lance sa synchronisation. */
  addAccount: (service: Service, username: string, apiKey: string | null) => invoke<Account>("add_account", { service, username, apiKey }),
  removeAccount: (id: number) => invoke<number>("remove_account", { id }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  startSync: (id: number | null = null) => invoke<void>("start_sync", { id }),
  stopSync: () => invoke<void>("stop_sync"),
  /** Écoutes des sources choisies (toutes si vide), fusionnées ; renvoie leur nombre. */
  exportHistory: (sources: number[], corrected: boolean, format: "csv" | "listenbrainz", path: string) =>
    invoke<number>("export_history", { sources, corrected, format, path }),
  mixes: (size: number) => invoke<Mix[]>("mixes", { size }),
  mixAt: (key: string, size: number) => invoke<Mix | null>("mix_at", { key, size }),
  seededMix: (kind: Kind, id: number, variant: number, size: number) => invoke<Mix | null>("seeded_mix", { kind, id, variant, size }),
  /** Retire un titre d'un mix ; mixKey = "*" : ne plus jamais le proposer. */
  mixFeedback: (mixKey: string, trackId: number) => invoke<void>("mix_feedback", { mixKey, trackId }),
  clearMixFeedback: () => invoke<number>("clear_mix_feedback"),
  discoverSelection: (scope: DiscoverScope) => invoke<Selection>("discover_selection", { scope }),
  startDiscover: (scope: DiscoverScope) => invoke<void>("start_discover", { scope }),
  stopDiscover: () => invoke<void>("stop_discover"),
  discoverRunning: () => invoke<boolean>("discover_running"),
  discoverVerdict: (deezerId: number, artist: string, verdict: number) => invoke<void>("discover_verdict", { deezerId, artist, verdict }),
  discoverPreview: (deezerId: number) => invoke<string | null>("discover_preview", { deezerId }),
  exportMix: (mix: Mix, title: string, format: MixFormat, path: string) => invoke<void>("export_mix", { mix, title, format, path }),
};

export type DiscoverScope = { kind: "weekly" } | { kind: "artist"; id: number };

export interface Found {
  deezer_id: number;
  title: string;
  artist: string;
  album: string | null;
  cover: string | null;
  duration_s: number | null;
  because: string[];
  sources: string[];
  score: number;
}

export interface Selection {
  new_artists: Found[];
  known_artists: Found[];
  computed_at: number | null;
}

export interface DiscoverStatus {
  running: boolean;
  done: number;
  total: number;
  current: string | null;
}

export interface MixTrack {
  id: number;
  title: string;
  artist: string;
  artist_id: number;
  album_id: number | null;
  album: string | null;
  plays: number;
  duration_ms: number | null;
}

export interface Mix {
  /** Stable d'une semaine à l'autre : « ambiance:… », « moment:… », « around:… »… */
  key: string;
  kind: "ambiance" | "moment" | "season" | "rediscover" | "around" | "gems" | "seed";
  params: Record<string, string>;
  tracks: MixTrack[];
  covers: number[];
  duration_ms: number;
}

export type Service = "librefm" | "lastfm" | "listenbrainz";

export interface Account {
  id: number;
  service: Service;
  username: string;
  source_id: number;
  synced_until: number | null;
  /** Synchronisation interrompue : [début, position, fin]. */
  pending: [number, number, number] | null;
  synced_at: number | null;
  error: string | null;
  plays: number;
  exclusive: number;
}

export interface SyncStatus {
  running: boolean;
  account_id: number | null;
  label: string | null;
  progress: number;
  fetched: number;
  added: number;
  merged: number;
  throttled_until: number;
}

export type MixFormat = "xspf" | "jspf" | "csv";

export function entityHref(kind: Kind, id: number): string {
  return `/entity?kind=${kind}&id=${id}`;
}

/** Une personne créditée : sa fiche artiste si elle en a une, sinon sa page personne. */
export function personHref(personId: number, artistId: number | null | undefined): string {
  return artistId != null ? entityHref("artist", artistId) : `/person?id=${personId}`;
}
