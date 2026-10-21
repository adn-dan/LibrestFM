package io.github.adn_dan.librestfm

import android.content.Context
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import org.json.JSONArray
import org.json.JSONObject

/**
 * Comptes vers lesquels les écoutes du téléphone sont envoyées, chacun avec sa file
 * d'attente (fichier JSON Lines) : une écoute reste en file tant que le service ne l'a
 * pas acceptée (hors ligne, serveur indisponible), puis part au retour du réseau.
 *
 * Protocoles : Last.fm et GNU FM (Libre.fm ou serveur perso) par l'API 2.0 ;
 * ListenBrainz (ou serveur compatible : Maloja, Koito…) par son API.
 * Mot de passe jamais gardé : seule la clé de session (ou le jeton) l'est.
 */
object Destinations {
  const val LASTFM = "lastfm"
  const val GNUFM = "gnufm"
  const val LISTENBRAINZ = "listenbrainz"
  private const val LASTFM_URL = "https://ws.audioscrobbler.com/2.0/"
  const val LIBREFM_URL = "https://libre.fm/2.0/"
  const val LISTENBRAINZ_URL = "https://api.listenbrainz.org"
  private const val BATCH = 50
  private const val USER_AGENT = "LibrestFM (Android)"

  /** Un seul fil pour les files et les envois : pas d'écriture concurrente. */
  private val worker = Executors.newSingleThreadExecutor()

  class AuthError(msg: String) : Exception(msg)

  private fun prefs(ctx: Context) = ctx.getSharedPreferences("destinations", Context.MODE_PRIVATE)

  fun list(ctx: Context): List<JSONObject> {
    val a = JSONArray(prefs(ctx).getString("list", "[]"))
    return (0 until a.length()).map { a.getJSONObject(it) }
  }

  @Synchronized
  private fun save(ctx: Context, list: List<JSONObject>) = prefs(ctx).edit().putString("list", JSONArray(list).toString()).apply()

  @Synchronized
  private fun update(ctx: Context, id: String, f: (JSONObject) -> Unit) = save(ctx, list(ctx).onEach { if (it.getString("id") == id) f(it) })

  /** Enregistrement local (journal CSV) : actif par défaut. */
  fun local(ctx: Context) = prefs(ctx).getBoolean("local", true)

  fun setLocal(ctx: Context, on: Boolean) = prefs(ctx).edit().putBoolean("local", on).apply()

  private fun queue(ctx: Context, id: String) = File(File(ctx.filesDir, "queue").apply { mkdirs() }, "$id.jsonl")

  fun pending(ctx: Context, id: String): Int = queue(ctx, id).takeIf { it.exists() }?.readLines()?.count { it.isNotBlank() } ?: 0

  /** Vue pour l'interface : sans clé de session ni jeton. */
  fun public(ctx: Context, d: JSONObject): JSONObject = JSONObject()
    .put("id", d.getString("id"))
    .put("kind", d.getString("kind"))
    .put("url", d.optString("url"))
    .put("user", d.optString("user"))
    .put("enabled", d.optBoolean("enabled", true))
    .put("error", d.optString("error").ifEmpty { null })
    .put("pending", pending(ctx, d.getString("id")))

  private fun md5(s: String) = MessageDigest.getInstance("MD5").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

  private fun enc(s: String) = URLEncoder.encode(s, "UTF-8")

  private fun http(url: String, body: String, contentType: String, auth: String? = null): Pair<Int, String> {
    val c = URL(url).openConnection() as HttpURLConnection
    try {
      c.connectTimeout = 20_000
      c.readTimeout = 30_000
      c.requestMethod = "POST"
      c.doOutput = true
      c.setRequestProperty("Content-Type", contentType)
      c.setRequestProperty("User-Agent", USER_AGENT)
      auth?.let { c.setRequestProperty("Authorization", it) }
      c.outputStream.use { it.write(body.toByteArray()) }
      val code = c.responseCode
      val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() } ?: ""
      return code to text
    } finally {
      c.disconnect()
    }
  }

  /** Appel signé de l'API 2.0 (Last.fm, GNU FM). */
  private fun audioscrobbler(base: String, params: Map<String, String>, secret: String): JSONObject {
    val sig = md5(params.toSortedMap().entries.joinToString("") { it.key + it.value } + secret)
    val body = (params + mapOf("api_sig" to sig, "format" to "json")).entries.joinToString("&") { enc(it.key) + "=" + enc(it.value) }
    val (code, text) = http(base, body, "application/x-www-form-urlencoded")
    val json = try {
      JSONObject(text)
    } catch (_: Exception) {
      throw Exception("HTTP $code")
    }
    if (json.has("error")) {
      val msg = json.optString("message", "error ${json.optInt("error")}")
      // 4 : identifiants, 9 : session expirée, 10/26 : clé d'API.
      if (json.optInt("error") in setOf(4, 9, 10, 26)) throw AuthError(msg) else throw Exception(msg)
    }
    return json
  }

  /** Connexion : vérifie les identifiants et ajoute le compte (actif). Réseau : hors du fil principal. */
  fun add(ctx: Context, kind: String, url: String?, user: String, secretInput: String, apiKey: String?, apiSecret: String?): JSONObject {
    val d = JSONObject().put("id", UUID.randomUUID().toString()).put("kind", kind).put("user", user).put("enabled", true)
    when (kind) {
      LASTFM -> {
        val key = apiKey?.trim().orEmpty()
        val secret = apiSecret?.trim().orEmpty()
        val sk = audioscrobbler(LASTFM_URL, mapOf("method" to "auth.getMobileSession", "username" to user, "password" to secretInput, "api_key" to key), secret)
          .getJSONObject("session").getString("key")
        d.put("url", LASTFM_URL).put("key", key).put("secret", secret).put("sk", sk)
      }
      GNUFM -> {
        // GNU FM n'impose pas de clé d'API : n'importe laquelle, et le jeton md5(user + md5(mot de passe)).
        val base = (url?.trim()?.ifEmpty { null } ?: LIBREFM_URL).let { if (it.endsWith("/")) it else "$it/" }
        val key = md5("librestfm")
        val session = audioscrobbler(base, mapOf("method" to "auth.getMobileSession", "username" to user, "authToken" to md5(user + md5(secretInput)), "api_key" to key), key)
          .getJSONObject("session")
        d.put("url", base).put("key", key).put("secret", key).put("sk", session.getString("key"))
      }
      LISTENBRAINZ -> {
        val base = (url?.trim()?.ifEmpty { null } ?: LISTENBRAINZ_URL).trimEnd('/')
        val c = URL("$base/1/validate-token").openConnection() as HttpURLConnection
        val ok = try {
          c.setRequestProperty("Authorization", "Token ${secretInput.trim()}")
          c.setRequestProperty("User-Agent", USER_AGENT)
          c.responseCode == 200 && JSONObject(c.inputStream.bufferedReader().readText()).optBoolean("valid")
        } finally {
          c.disconnect()
        }
        if (!ok) throw AuthError("invalid token")
        d.put("url", base).put("token", secretInput.trim())
      }
      else -> throw Exception("kind")
    }
    save(ctx, list(ctx) + d)
    return d
  }

  fun remove(ctx: Context, id: String) {
    save(ctx, list(ctx).filter { it.getString("id") != id })
    worker.execute { queue(ctx, id).delete() }
  }

  fun setEnabled(ctx: Context, id: String, on: Boolean) = update(ctx, id) { it.put("enabled", on) }

  /** Ajoute l'écoute à la file de chaque compte actif, puis tente l'envoi. */
  fun enqueue(ctx: Context, item: JSONObject) {
    val ids = list(ctx).filter { it.optBoolean("enabled", true) }.map { it.getString("id") }
    if (ids.isEmpty()) return
    worker.execute { ids.forEach { queue(ctx, it).appendText(item.toString() + "\n") } }
    flush(ctx)
  }

  /** Envoie les files en attente (au retour du réseau, après chaque écoute, à l'ouverture). */
  fun flush(ctx: Context) {
    worker.execute {
      for (d in list(ctx)) {
        if (!d.optBoolean("enabled", true)) continue
        val f = queue(ctx, d.getString("id"))
        if (!f.exists()) continue
        try {
          while (true) {
            val lines = f.readLines().filter { it.isNotBlank() }
            if (lines.isEmpty()) break
            send(d, lines.take(BATCH).map(::JSONObject))
            f.writeText(lines.drop(BATCH).joinToString("") { it + "\n" })
          }
          if (d.has("error")) update(ctx, d.getString("id")) { it.remove("error") }
        } catch (e: AuthError) {
          update(ctx, d.getString("id")) { it.put("error", "auth") }
        } catch (e: Exception) {
          // Réseau ou serveur : la file reste, nouvel essai plus tard.
          update(ctx, d.getString("id")) { it.put("error", "network") }
        }
      }
    }
  }

  private fun send(d: JSONObject, items: List<JSONObject>) {
    when (d.getString("kind")) {
      LISTENBRAINZ -> {
        val payload = JSONArray(items.map { i ->
          val info = JSONObject().put("submission_client", "LibrestFM").put("media_player", i.optString("player"))
          if (i.optLong("durationMs") > 0) info.put("duration_ms", i.optLong("durationMs"))
          val meta = JSONObject().put("artist_name", i.getString("artist")).put("track_name", i.getString("track")).put("additional_info", info)
          i.optString("album").ifEmpty { null }?.let { meta.put("release_name", it) }
          JSONObject().put("listened_at", i.getLong("uts")).put("track_metadata", meta)
        })
        val body = JSONObject().put("listen_type", if (items.size == 1) "single" else "import").put("payload", payload).toString()
        val (code, text) = http(d.getString("url") + "/1/submit-listens", body, "application/json", "Token " + d.getString("token"))
        if (code == 401) throw AuthError(text)
        if (code != 200) throw Exception("HTTP $code")
      }
      else -> {
        val p = mutableMapOf("method" to "track.scrobble", "api_key" to d.getString("key"), "sk" to d.getString("sk"))
        items.forEachIndexed { n, i ->
          p["artist[$n]"] = i.getString("artist")
          p["track[$n]"] = i.getString("track")
          p["timestamp[$n]"] = i.getLong("uts").toString()
          i.optString("album").ifEmpty { null }?.let { p["album[$n]"] = it }
          i.optString("albumArtist").ifEmpty { null }?.let { p["albumArtist[$n]"] = it }
          if (i.optLong("durationMs") > 0) p["duration[$n]"] = (i.optLong("durationMs") / 1000).toString()
        }
        // Écoutes refusées par le service (trop anciennes…) : retirées de la file quand même.
        audioscrobbler(d.getString("url"), p, d.getString("secret"))
      }
    }
  }
}
