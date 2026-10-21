package io.github.adn_dan.librestfm

import android.Manifest
import android.app.Activity
import android.media.MediaMetadata
import android.media.session.PlaybackState
import android.os.Build
import android.os.SystemClock
import android.provider.OpenableColumns
import java.io.File
import androidx.core.app.ActivityCompat
import android.graphics.Color
import androidx.core.view.WindowCompat
import android.view.WindowManager
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class DirArgs {
  lateinit var dir: String
}

@InvokeArg
class AppArgs {
  lateinit var pkg: String
  var enabled: Boolean = false
}

@InvokeArg
class DestArgs {
  lateinit var kind: String
  var url: String? = null
  var user: String = ""
  /** Mot de passe (Last.fm, GNU FM, utilisé une fois) ou jeton (ListenBrainz). */
  lateinit var secret: String
  var apiKey: String? = null
  var apiSecret: String? = null
}

@InvokeArg
class IdArgs {
  lateinit var id: String
  var enabled: Boolean = false
}

@InvokeArg
class NotifArgs {
  var active: Boolean = false
  var jobs: Boolean = false
  var labels: Map<String, String>? = null
  /** Choix de l'utilisateur : demander l'autorisation si elle manque. */
  var ask: Boolean = false
}

@InvokeArg
class ProgressArgs {
  var text: String? = null
  var percent: Int? = null
}

@InvokeArg
class UriArgs {
  lateinit var uri: String
  var from: String? = null
}

@InvokeArg
class NameArgs {
  lateinit var name: String
}

@InvokeArg
class ControlArgs {
  /** play, pause, next, previous */
  lateinit var action: String
}

@InvokeArg
class OnArgs {
  var enabled: Boolean = false
}

/** Pont entre l'interface et le scrobbleur : accès aux notifications, applis suivies, journal. */
@TauriPlugin
class ScrobblerPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun configure(invoke: Invoke) {
    ScrobbleStore.setDir(activity, invoke.parseArgs(DirArgs::class.java).dir)
    // À l'ouverture : écoutes restées en file envoyées.
    Destinations.flush(activity)
    invoke.resolve()
  }

  @Command
  fun status(invoke: Invoke) {
    val enabled = ScrobbleStore.enabled(activity)
    val seen = ScrobbleStore.seen(activity)
    val apps = JSArray()
    ScrobbleStore.candidates(activity)
      .map { Triple(it, ScrobbleStore.label(activity, it), it in enabled) }
      .sortedWith(compareBy({ !it.third }, { it.second.lowercase() }))
      .forEach { (pkg, label, on) ->
        apps.put(JSObject().put("pkg", pkg).put("label", label).put("enabled", on).put("seen", pkg in seen))
      }
    val res = JSObject()
      .put("access", NotificationManagerCompat.getEnabledListenerPackages(activity).contains(activity.packageName))
      .put("apps", apps)
      .put("count", ScrobbleStore.count(activity))
      .put("local", Destinations.local(activity))
      .put("notif", JSObject().put("active", Notifs.active(activity)).put("jobs", Notifs.jobs(activity)).put("allowed", Notifs.allowed(activity)))
      .put("destinations", JSArray().apply { Destinations.list(activity).forEach { put(Destinations.public(activity, it)) } })
    ScrobbleStore.last(activity)?.let { (artist, track, uts) ->
      res.put("last", JSObject().put("artist", artist).put("track", track).put("uts", uts))
    }
    invoke.resolve(res)
  }

  @Command
  fun setApp(invoke: Invoke) {
    val args = invoke.parseArgs(AppArgs::class.java)
    ScrobbleStore.setEnabled(activity, args.pkg, args.enabled)
    invoke.resolve()
  }

  /** Réglage système de l'accès aux notifications. */
  @Command
  fun openAccess(invoke: Invoke) {
    activity.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
    invoke.resolve()
  }

  /** Fiche de l'app : « Autoriser les paramètres restreints » (Android 13+, app installée hors magasin). */
  @Command
  fun openAppInfo(invoke: Invoke) {
    activity.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", activity.packageName, null)))
    invoke.resolve()
  }

  /** Barres système aux couleurs du thème choisi dans l'app (qui peut différer de celui du téléphone). */
  @Command
  fun setBars(invoke: Invoke) {
    val dark = invoke.parseArgs(OnArgs::class.java).enabled
    activity.runOnUiThread {
      activity.window.decorView.setBackgroundColor(Color.parseColor(if (dark) "#1a1d23" else "#ffffff"))
      WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
        isAppearanceLightStatusBars = !dark
        isAppearanceLightNavigationBars = !dark
      }
    }
    invoke.resolve()
  }

  /** Écran allumé (paroles qui défilent pendant la lecture) : pas de mise en veille. */
  @Command
  fun keepAwake(invoke: Invoke) {
    val on = invoke.parseArgs(OnArgs::class.java).enabled
    activity.runOnUiThread {
      if (on) activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      else activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }
    invoke.resolve()
  }

  /** Options des notifications et leurs textes ; demande l'autorisation (Android 13+) si besoin. */
  @Command
  fun setNotifs(invoke: Invoke) {
    val a = invoke.parseArgs(NotifArgs::class.java)
    Notifs.configure(activity, a.active, a.jobs, a.labels?.let { org.json.JSONObject(it) })
    if (a.ask && (a.active || a.jobs) && !Notifs.allowed(activity) && Build.VERSION.SDK_INT >= 33) {
      ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
    }
    invoke.resolve()
  }

  @Command
  fun progress(invoke: Invoke) {
    val a = invoke.parseArgs(ProgressArgs::class.java)
    Notifs.progress(activity, a.text, a.percent)
    invoke.resolve()
  }

  /** Titre en cours (toute appli) et position de lecture estimée à l'instant. */
  @Command
  fun nowPlaying(invoke: Invoke) {
    val res = JSObject().put("access", ScrobbleService.connected)
    val c = ScrobbleService.now
    val m = c?.metadata
    val s = c?.playbackState
    if (c != null && m != null) {
      val playing = s?.state == PlaybackState.STATE_PLAYING
      var pos = s?.position ?: 0
      if (playing && s != null && s.lastPositionUpdateTime > 0) {
        pos += ((SystemClock.elapsedRealtime() - s.lastPositionUpdateTime) * s.playbackSpeed).toLong()
      }
      val artist = m.getString(MediaMetadata.METADATA_KEY_ARTIST)?.ifBlank { null } ?: m.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST)
      res.put("track", JSObject()
        .put("artist", artist ?: "")
        .put("title", m.getString(MediaMetadata.METADATA_KEY_TITLE) ?: "")
        .put("album", m.getString(MediaMetadata.METADATA_KEY_ALBUM))
        .put("albumArtist", m.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST))
        .put("durationMs", m.getLong(MediaMetadata.METADATA_KEY_DURATION))
        .put("positionMs", pos)
        .put("playing", playing)
        .put("player", ScrobbleStore.label(activity, c.packageName)))
    }
    invoke.resolve(res)
  }

  /** Commande le lecteur du titre en cours, sans changer d'appli. */
  @Command
  fun mediaControl(invoke: Invoke) {
    val c = ScrobbleService.now ?: return invoke.reject("none")
    val t = c.transportControls
    when (invoke.parseArgs(ControlArgs::class.java).action) {
      "play" -> t.play()
      "pause" -> t.pause()
      "next" -> t.skipToNext()
      "previous" -> t.skipToPrevious()
    }
    invoke.resolve()
  }

  /** Page demandée par une notification touchée (lue une seule fois). */
  @Command
  fun takeOpen(invoke: Invoke) {
    val p = MainActivity.pendingOpen
    MainActivity.pendingOpen = null
    invoke.resolve(JSObject().put("page", p))
  }

  // Fichiers choisis dans le sélecteur d'Android : des adresses content://, que le cœur
  // ne sait pas ouvrir. Lecture : copie locale ; écriture : fichier temporaire recopié.

  /** Copie locale d'un fichier choisi (gardant son nom, qui sert de nom de source). */
  @Command
  fun localCopy(invoke: Invoke) {
    val uri = Uri.parse(invoke.parseArgs(UriArgs::class.java).uri)
    Thread {
      try {
        val name = activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
          if (it.moveToFirst()) it.getString(0) else null
        }?.replace('/', '_') ?: "import"
        // Un dossier par fichier (plusieurs choisis d'un coup) ; ceux de plus d'une heure retirés.
        val root = File(activity.cacheDir, "picked")
        root.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > 3_600_000 }?.forEach { it.deleteRecursively() }
        val dir = File(root, java.util.UUID.randomUUID().toString()).apply { mkdirs() }
        val out = File(dir, name)
        activity.contentResolver.openInputStream(uri)!!.use { input -> out.outputStream().use { input.copyTo(it) } }
        invoke.resolve(JSObject().put("path", out.absolutePath))
      } catch (e: Exception) {
        invoke.reject(e.message ?: "read")
      }
    }.start()
  }

  /** Fichier temporaire où écrire avant la copie vers le fichier choisi. */
  @Command
  fun tempPath(invoke: Invoke) {
    val dir = File(activity.cacheDir, "out").apply { deleteRecursively(); mkdirs() }
    invoke.resolve(JSObject().put("path", File(dir, invoke.parseArgs(NameArgs::class.java).name.replace('/', '_')).absolutePath))
  }

  /** Recopie le fichier temporaire vers le fichier choisi (remplacé), puis le supprime. */
  @Command
  fun copyToUri(invoke: Invoke) {
    val a = invoke.parseArgs(UriArgs::class.java)
    Thread {
      val from = File(a.from ?: "")
      try {
        require(from.canonicalPath.startsWith(File(activity.cacheDir, "out").canonicalPath))
        activity.contentResolver.openOutputStream(Uri.parse(a.uri), "wt")!!.use { out -> from.inputStream().use { it.copyTo(out) } }
        invoke.resolve()
      } catch (e: Exception) {
        invoke.reject(e.message ?: "write")
      } finally {
        from.delete()
      }
    }.start()
  }

  @Command
  fun setLocal(invoke: Invoke) {
    Destinations.setLocal(activity, invoke.parseArgs(OnArgs::class.java).enabled)
    invoke.resolve()
  }

  /** Connexion au compte (réseau) : hors du fil principal. Erreur "auth" : identifiants refusés. */
  @Command
  fun addDestination(invoke: Invoke) {
    val a = invoke.parseArgs(DestArgs::class.java)
    Thread {
      try {
        Destinations.add(activity, a.kind, a.url, a.user.trim(), a.secret, a.apiKey, a.apiSecret)
        invoke.resolve()
      } catch (e: Destinations.AuthError) {
        invoke.reject("auth")
      } catch (e: Exception) {
        invoke.reject("network")
      }
    }.start()
  }

  @Command
  fun setDestination(invoke: Invoke) {
    val a = invoke.parseArgs(IdArgs::class.java)
    Destinations.setEnabled(activity, a.id, a.enabled)
    if (a.enabled) Destinations.flush(activity)
    invoke.resolve()
  }

  @Command
  fun removeDestination(invoke: Invoke) {
    Destinations.remove(activity, invoke.parseArgs(IdArgs::class.java).id)
    invoke.resolve()
  }

  @Command
  fun flush(invoke: Invoke) {
    Destinations.flush(activity)
    invoke.resolve()
  }
}
