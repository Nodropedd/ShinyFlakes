package com.shinyflakes.wallet

import android.os.Build
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebViewFeature

/**
 * The wallet itself. Reached from [StartupActivity], which has already proved
 * the Rust library loads and a WebView exists — the two things whose failure
 * here would leave a black screen with nothing to report.
 */
class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // Before super.onCreate, so it is in place before the WebView exists.
    // Here rather than in StartupActivity because Android can recreate this
    // activity directly, from recents after the process was killed.
    cutWebViewOffTheNetwork()

    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    // Nothing in this window is for autofill. The first thing it shows is a
    // box for a seed phrase, and Android offers every field on screen to the
    // user's autofill service — Google's or a password manager's — to read,
    // classify and offer to save. Autofill arrived in Android 8, so older
    // releases have nothing to exclude.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      window.decorView.importantForAutofill =
        View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
    }
  }

  /**
   * The WebView here never needs the network. The app's pages are served
   * in-process before any request reaches the network stack, calls into Rust
   * go through postMessage, and every wallet request is made by the Rust core
   * — through Tor when Tor is on.
   *
   * The WebView still makes requests of its own. Finding the seed-phrase box,
   * it sent the form's shape to content-autofill.googleapis.com, directly and
   * so around Tor, whatever the autofill settings said. Rather than chase each
   * such feature, point all of its traffic at a proxy address where nothing
   * listens, with no direct fallback: anything the WebView tries to fetch for
   * itself fails at once and never leaves the phone.
   *
   * Debug builds are exempt because `tauri android dev` loads the front end
   * from the Vite server over the network. A WebView too old for proxy
   * overrides (before 72) is left as it is.
   */
  private fun cutWebViewOffTheNetwork() {
    if (BuildConfig.DEBUG) return
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE)) return

    // Port 9 is "discard": reserved, and nothing on a phone listens there.
    val nowhere = ProxyConfig.Builder().addProxyRule("127.0.0.1:9").build()
    ProxyController.getInstance().setProxyOverride(nowhere, { it.run() }, {})
  }
}
