package io.github.adn_dan.librestfm

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import java.io.File

/** Réglages du scrobbleur (applis suivies) et journal des écoutes (CSV qui ne fait que grossir). */
object ScrobbleStore {
  private const val PREFS = "scrobbler"
  const val FILE = "scrobbles.csv"
  private const val HEADER = "uts,artist,track,album,album_artist,player\n"

  private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  /** Dossier du journal : celui que lit l'app (donné par le cœur au démarrage). */
  fun dir(ctx: Context): File = prefs(ctx).getString("dir", null)?.let(::File) ?: File(ctx.filesDir, "scrobbles")

  fun setDir(ctx: Context, dir: String) = prefs(ctx).edit().putString("dir", dir).apply()

  fun enabled(ctx: Context): Set<String> = prefs(ctx).getStringSet("enabled", emptySet())!!

  fun setEnabled(ctx: Context, pkg: String, on: Boolean) {
    val set = enabled(ctx).toMutableSet()
    if (on) set.add(pkg) else set.remove(pkg)
    prefs(ctx).edit().putStringSet("enabled", set).apply()
  }

  /** Applis vues en train de jouer quelque chose (proposées même sans service de lecture déclaré). */
  fun seen(ctx: Context): Set<String> = prefs(ctx).getStringSet("seen", emptySet())!!

  fun markSeen(ctx: Context, pkg: String) {
    val set = seen(ctx)
    if (pkg in set || pkg == ctx.packageName) return
    prefs(ctx).edit().putStringSet("seen", set + pkg).apply()
  }

  /** Lecteurs installés : applis qui exposent un service de navigation média, plus celles déjà vues. */
  fun candidates(ctx: Context): Set<String> {
    val pm = ctx.packageManager
    val browsers = pm.queryIntentServices(Intent("android.media.browse.MediaBrowserService"), 0).map { it.serviceInfo.packageName }
    val installed = (browsers + seen(ctx) + enabled(ctx)).filter {
      try {
        pm.getApplicationInfo(it, 0)
        true
      } catch (_: PackageManager.NameNotFoundException) {
        false
      }
    }
    return installed.toSet() - ctx.packageName
  }

  fun label(ctx: Context, pkg: String): String =
    try {
      ctx.packageManager.getApplicationLabel(ctx.packageManager.getApplicationInfo(pkg, 0)).toString()
    } catch (_: PackageManager.NameNotFoundException) {
      pkg
    }

  private fun cell(s: String?): String {
    val v = s ?: ""
    return if (v.any { it == ',' || it == '"' || it == '\n' || it == '\r' }) "\"" + v.replace("\"", "\"\"") + "\"" else v
  }

  @Synchronized
  fun append(ctx: Context, uts: Long, artist: String, track: String, album: String?, albumArtist: String?, player: String) {
    val dir = dir(ctx)
    dir.mkdirs()
    val f = File(dir, FILE)
    val line = listOf(uts.toString(), artist, track, album, albumArtist, player).joinToString(",", transform = ::cell) + "\n"
    f.appendText((if (f.exists() && f.length() > 0) "" else HEADER) + line)
    prefs(ctx).edit().putString("last_artist", artist).putString("last_track", track).putLong("last_uts", uts).apply()
  }

  /** Dernière écoute enregistrée : (artiste, titre, heure Unix). */
  fun last(ctx: Context): Triple<String, String, Long>? {
    val p = prefs(ctx)
    val a = p.getString("last_artist", null) ?: return null
    return Triple(a, p.getString("last_track", "")!!, p.getLong("last_uts", 0))
  }

  /** Nombre d'écoutes enregistrées dans le journal. */
  fun count(ctx: Context): Int {
    val f = File(dir(ctx), FILE)
    if (!f.exists()) return 0
    var n = 0
    f.forEachLine { n++ }
    return (n - 1).coerceAtLeast(0)
  }
}
