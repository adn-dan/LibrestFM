package io.github.adn_dan.librestfm

import android.content.ComponentName
import android.net.ConnectivityManager
import android.net.Network
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSession
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.service.notification.NotificationListenerService
import org.json.JSONObject

/**
 * Détecte la musique jouée par les autres applis (sessions média, lisibles grâce à
 * l'accès aux notifications) et l'ajoute au journal. Aucun service au premier plan :
 * le système garde ce service lié tant que l'accès est accordé, même app fermée.
 *
 * Chaque écoute va au journal local (si choisi) et dans la file de chaque compte actif.
 *
 * Règle d'enregistrement (celle de Last.fm) : titre de plus de 30 s, écouté la moitié
 * de sa durée ou 4 min. Durée inconnue : 30 s d'écoute. Heure retenue : le début.
 */
class ScrobbleService : NotificationListenerService() {
  companion object {
    /** Relié au système (accès aux notifications accordé) : le scrobbleur fonctionne. */
    @Volatile var connected = false
    /** Dernière session passée en lecture (toute appli) : titre et position pour les paroles. */
    @Volatile var now: MediaController? = null
  }

  private val handler = Handler(Looper.getMainLooper())
  private val sessions = HashMap<MediaSession.Token, Session>()
  private var manager: MediaSessionManager? = null
  private val onChange = MediaSessionManager.OnActiveSessionsChangedListener { list -> follow(list.orEmpty()) }
  /** Retour du réseau : les écoutes en attente partent vers les comptes. */
  private val onNetwork = object : ConnectivityManager.NetworkCallback() {
    override fun onAvailable(network: Network) = Destinations.flush(this@ScrobbleService)
  }

  override fun onListenerConnected() {
    connected = true
    Notifs.refreshActive(this)
    val m = getSystemService(MediaSessionManager::class.java) ?: return
    manager = m
    val me = ComponentName(this, ScrobbleService::class.java)
    m.addOnActiveSessionsChangedListener(onChange, me, handler)
    follow(m.getActiveSessions(me))
    getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(onNetwork)
  }

  override fun onListenerDisconnected() {
    connected = false
    Notifs.playing = null
    Notifs.refreshActive(this)
    manager?.removeOnActiveSessionsChangedListener(onChange)
    runCatching { getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(onNetwork) }
    sessions.values.forEach { it.release() }
    sessions.clear()
  }

  private fun follow(list: List<MediaController>) {
    val tokens = list.map { it.sessionToken }.toSet()
    sessions.keys.filter { it !in tokens }.forEach { sessions.remove(it)?.release() }
    for (c in list) {
      if (c.sessionToken in sessions) continue
      ScrobbleStore.markSeen(this, c.packageName)
      sessions[c.sessionToken] = Session(c)
    }
    // Rien joué depuis la connexion : session la plus récente avec un titre (en pause, par exemple).
    if (now == null) now = list.firstOrNull { it.metadata?.getString(MediaMetadata.METADATA_KEY_TITLE) != null }
  }

  private data class Track(val artist: String, val title: String, val album: String?, val albumArtist: String?, val durationMs: Long) {
    fun same(o: Track?) = o != null && o.artist == artist && o.title == title && o.album == album
  }

  private fun trackOf(m: MediaMetadata?): Track? {
    m ?: return null
    val title = m.getString(MediaMetadata.METADATA_KEY_TITLE)?.trim().orEmpty()
    val albumArtist = m.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST)?.trim()?.ifEmpty { null }
    val artist = m.getString(MediaMetadata.METADATA_KEY_ARTIST)?.trim()?.ifEmpty { null } ?: albumArtist ?: return null
    if (title.isEmpty()) return null
    val album = m.getString(MediaMetadata.METADATA_KEY_ALBUM)?.trim()?.ifEmpty { null }
    return Track(artist, title, album, albumArtist, m.getLong(MediaMetadata.METADATA_KEY_DURATION))
  }

  private inner class Session(private val c: MediaController) : MediaController.Callback() {
    private var track: Track? = null
    private var startUts = 0L
    private var playedMs = 0L
    private var resumedAt = -1L
    private var done = false
    private val check = Runnable { maybeScrobble() }

    init {
      c.registerCallback(this, handler)
      onMetadataChanged(c.metadata)
      onPlaybackStateChanged(c.playbackState)
    }

    fun release() {
      pause()
      c.unregisterCallback(this)
    }

    override fun onSessionDestroyed() {
      if (now?.sessionToken == c.sessionToken) now = null
      release()
      sessions.remove(c.sessionToken)
    }

    override fun onMetadataChanged(metadata: MediaMetadata?) {
      val t = trackOf(metadata) ?: return
      if (t.same(track)) {
        // Durée parfois connue après coup.
        if (t.durationMs > 0 && track!!.durationMs <= 0) track = t
        return
      }
      val playing = resumedAt >= 0
      pause()
      track = t
      startUts = System.currentTimeMillis() / 1000
      playedMs = 0
      done = false
      if (playing) resume()
    }

    override fun onPlaybackStateChanged(state: PlaybackState?) {
      if (state?.state == PlaybackState.STATE_PLAYING) {
        // Même titre relancé depuis le début (répétition) : nouvelle écoute.
        if (done && state.position in 0..3000) {
          pause()
          startUts = System.currentTimeMillis() / 1000
          playedMs = 0
          done = false
        }
        resume()
      } else {
        pause()
      }
    }

    private fun needMs(): Long? {
      val d = track?.durationMs ?: return null
      return when {
        d <= 0 -> 30_000
        d <= 30_000 -> null
        else -> minOf(d / 2, 240_000)
      }
    }

    /** Notification « scrobbleur actif » : titre en cours, s'il vient d'une appli suivie. */
    private fun showPlaying(on: Boolean) {
      val t = track ?: return
      val ctx = this@ScrobbleService
      if (on && c.packageName in ScrobbleStore.enabled(ctx)) {
        Notifs.playing = t.artist to t.title
        Notifs.player = ScrobbleStore.label(ctx, c.packageName)
      }
      else if (!on && Notifs.playing == (t.artist to t.title)) Notifs.playing = null
      else return
      Notifs.refreshActive(ctx)
    }

    private fun resume() {
      now = c
      if (resumedAt >= 0 || track == null) return
      showPlaying(true)
      if (done) return
      resumedAt = SystemClock.elapsedRealtime()
      val need = needMs() ?: return
      handler.postDelayed(check, (need - playedMs).coerceAtLeast(0) + 500)
    }

    private fun pause() {
      handler.removeCallbacks(check)
      showPlaying(false)
      if (resumedAt < 0) return
      playedMs += SystemClock.elapsedRealtime() - resumedAt
      resumedAt = -1
    }

    private fun maybeScrobble() {
      val t = track ?: return
      val need = needMs() ?: return
      val played = playedMs + if (resumedAt >= 0) SystemClock.elapsedRealtime() - resumedAt else 0
      if (done) return
      if (played < need) {
        handler.postDelayed(check, need - played + 500)
        return
      }
      done = true
      val ctx = this@ScrobbleService
      if (c.packageName !in ScrobbleStore.enabled(ctx)) return
      if (Destinations.local(ctx)) ScrobbleStore.append(ctx, startUts, t.artist, t.title, t.album, t.albumArtist, c.packageName)
      Destinations.enqueue(
        ctx,
        JSONObject().put("uts", startUts).put("artist", t.artist).put("track", t.title).put("album", t.album ?: "")
          .put("albumArtist", t.albumArtist ?: "").put("durationMs", t.durationMs).put("player", c.packageName),
      )
    }
  }
}
