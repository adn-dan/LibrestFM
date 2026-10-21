package io.github.adn_dan.librestfm

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONObject

/**
 * Notifications facultatives, toutes silencieuses : scrobbleur actif (permanente, titre en
 * cours) et avancement des téléchargements (synchro, enrichissement). Les textes viennent de
 * l'interface, dans la langue choisie dans l'app.
 */
object Notifs {
  private const val CH_ACTIVE = "scrobbler"
  private const val CH_JOBS = "jobs"
  private const val ID_ACTIVE = 1
  private const val ID_JOBS = 2

  /** Titre suivi en ce moment (artiste, titre), mis à jour par le service. */
  @Volatile var playing: Pair<String, String>? = null
  /** Appli qui le joue (nom affiché). */
  @Volatile var player: String? = null

  private fun prefs(ctx: Context) = ctx.getSharedPreferences("notifs", Context.MODE_PRIVATE)

  fun active(ctx: Context) = prefs(ctx).getBoolean("active", false)

  fun jobs(ctx: Context) = prefs(ctx).getBoolean("jobs", false)

  fun allowed(ctx: Context) = NotificationManagerCompat.from(ctx).areNotificationsEnabled()

  /** Options et textes (clés : activeTitle, idle, playing avec {track} et {artist}, jobsTitle). */
  fun configure(ctx: Context, active: Boolean, jobs: Boolean, labels: JSONObject?) {
    val e = prefs(ctx).edit().putBoolean("active", active).putBoolean("jobs", jobs)
    labels?.keys()?.forEach { e.putString("label.$it", labels.getString(it)) }
    e.apply()
    channels(ctx)
    if (!jobs) NotificationManagerCompat.from(ctx).cancel(ID_JOBS)
    refreshActive(ctx)
  }

  private fun label(ctx: Context, key: String, fallback: String) = prefs(ctx).getString("label.$key", null) ?: fallback

  private fun channels(ctx: Context) {
    val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
    nm.createNotificationChannel(NotificationChannel(CH_ACTIVE, label(ctx, "activeTitle", "Scrobbler"), NotificationManager.IMPORTANCE_MIN).apply { setShowBadge(false) })
    nm.createNotificationChannel(NotificationChannel(CH_JOBS, label(ctx, "jobsTitle", "Downloads"), NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
  }

  private fun open(ctx: Context, page: String? = null): PendingIntent {
    val i = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    page?.let { i.putExtra("open", it) }
    return PendingIntent.getActivity(ctx, page?.hashCode() ?: 0, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
  }

  private fun base(ctx: Context, channel: String, page: String? = null) = NotificationCompat.Builder(ctx, channel)
    .setSmallIcon(R.drawable.ic_stat_note)
    .setContentIntent(open(ctx, page))
    .setSilent(true)
    .setOnlyAlertOnce(true)
    .setShowWhen(false)

  /** Scrobbleur actif : affichée tant que le service est relié et l'option cochée. */
  fun refreshActive(ctx: Context, connected: Boolean = ScrobbleService.connected) {
    val nm = NotificationManagerCompat.from(ctx)
    if (!active(ctx) || !connected || !allowed(ctx)) {
      nm.cancel(ID_ACTIVE)
      return
    }
    channels(ctx)
    val text = playing?.let { (artist, track) ->
      label(ctx, "playing", "{track} · {artist}").replace("{track}", track).replace("{artist}", artist)
    } ?: label(ctx, "idle", "")
    val n = base(ctx, CH_ACTIVE, "now")
      .setContentTitle(label(ctx, "activeTitle", "Scrobbler"))
      .setContentText(text)
      .setSubText(if (playing != null) player else null)
      .setOngoing(true)
      .setPriority(NotificationCompat.PRIORITY_MIN)
      .build()
    try {
      nm.notify(ID_ACTIVE, n)
    } catch (_: SecurityException) {
    }
  }

  /** Avancement des téléchargements ; text null : terminé, notification retirée. */
  fun progress(ctx: Context, text: String?, percent: Int?) {
    val nm = NotificationManagerCompat.from(ctx)
    if (text == null || !jobs(ctx) || !allowed(ctx)) {
      nm.cancel(ID_JOBS)
      return
    }
    channels(ctx)
    val n = base(ctx, CH_JOBS)
      .setContentTitle(label(ctx, "jobsTitle", "Downloads"))
      .setContentText(text)
      .setStyle(NotificationCompat.BigTextStyle().bigText(text))
      .setProgress(100, percent ?: 0, percent == null)
      .setOngoing(true)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .build()
    try {
      nm.notify(ID_JOBS, n)
    } catch (_: SecurityException) {
    }
  }
}
