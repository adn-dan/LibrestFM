package io.github.adn_dan.librestfm

import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  companion object {
    /** Page à ouvrir, demandée par une notification (« now » : en écoute) ; lue une fois par l'interface. */
    @Volatile var pendingOpen: String? = null
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    intent.getStringExtra("open")?.let { pendingOpen = it }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    intent?.getStringExtra("open")?.let { pendingOpen = it }
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // L'interface reste entre la barre d'état et la barre de navigation (et au-dessus du clavier).
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
    WindowCompat.getInsetsController(window, window.decorView).apply {
      isAppearanceLightStatusBars = !night
      isAppearanceLightNavigationBars = !night
    }
  }

  // Interface plus compacte que le réglage par défaut du système (88 %), sans zoom
  // au pincement ni au double appui : c'est une appli, pas une page web.
  override fun onWebViewCreate(webView: WebView) {
    webView.setInitialScale((resources.displayMetrics.density * 88).toInt())
    webView.settings.setSupportZoom(false)
    webView.settings.builtInZoomControls = false
    webView.settings.displayZoomControls = false
  }
}
