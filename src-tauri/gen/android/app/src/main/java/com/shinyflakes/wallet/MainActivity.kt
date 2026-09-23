// Wallet activity
package com.shinyflakes.wallet

import android.graphics.Bitmap
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.PixelCopy
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import androidx.webkit.WebViewRenderProcess
import androidx.webkit.WebViewRenderProcessClient
import org.json.JSONTokener

class MainActivity : TauriActivity() {
  private val main = Handler(Looper.getMainLooper())
  private val startedAt = SystemClock.uptimeMillis()
  private var webView: WebView? = null
  private var rendererHung = false
  private var networkLocked = false
  private var report: View? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    Diagnostics.stage("MainActivity onCreate")
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    Diagnostics.stage("wallet activity created")

    // no autofill
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      window.decorView.importantForAutofill =
        View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
    }

    if (Diagnostics.takeTestHang()) {
      Diagnostics.stage("test freeze")
      SystemClock.sleep(15_000)
    }

    // recreated without the launcher after a failed start
    if (!Diagnostics.launcherRan) {
      Diagnostics.previousRunReport(this)?.let { previous ->
        Diagnostics.stage(Diagnostics.REPORT_SHOWN)
        report = Diagnostics.reportView(this, "ShinyFlakes didn't start last time", previous,
          listOf("Continue" to { dismissReport() })).also {
          addContentView(it, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        }
      }
    }

    val force = intent?.getBooleanExtra(Diagnostics.FORCE_EXTRA, false) == true
    main.postDelayed({ checkStarted(final = false, force = force) }, FIRST_CHECK_MS)
  }

  override fun onWebViewCreate(webView: WebView) {
    Diagnostics.stage("WebView created")
    this.webView = webView
    try {
      cutWebViewOffTheNetwork()
    } catch (t: Throwable) {
      Diagnostics.stage("network lock failed: $t")
    }
    if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_VIEW_RENDERER_CLIENT_BASIC_USAGE)) {
      WebViewCompat.setWebViewRenderProcessClient(webView, object : WebViewRenderProcessClient() {
        override fun onRenderProcessUnresponsive(view: WebView, renderer: WebViewRenderProcess?) {
          rendererHung = true
          Diagnostics.stage("WebView renderer unresponsive")
        }
        override fun onRenderProcessResponsive(view: WebView, renderer: WebViewRenderProcess?) {
          rendererHung = false
        }
      })
    }
  }

  // network lock
  private fun cutWebViewOffTheNetwork() {
    if (BuildConfig.DEBUG) return
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE)) return
    val nowhere = ProxyConfig.Builder().addProxyRule("127.0.0.1:9").build()
    ProxyController.getInstance().setProxyOverride(nowhere, { it.run() }, {})
    networkLocked = true
  }

  // start-up watchdog
  private fun checkStarted(final: Boolean, force: Boolean) {
    if (isFinishing || report != null) return
    val retry = { main.postDelayed({ checkStarted(final = true, force = force) }, FINAL_CHECK_MS - FIRST_CHECK_MS) }

    val wv = webView
    if (wv == null) {
      Diagnostics.stage("no WebView yet at ${elapsed()} ms")
      if (final) showReport("The wallet core never created its window", "Rust did not hand the app a WebView within ${elapsed()} ms.")
      else retry()
      return
    }

    var answered = false
    wv.evaluateJavascript(PROBE) { raw ->
      answered = true
      val page = decode(raw)
      val native = "WebView url=${wv.url} progress=${wv.progress}%"
      if (!page.startsWith("MOUNTED")) {
        Diagnostics.stage("page not mounted at ${elapsed()} ms")
        if (final) showReport("The page loaded but the app did not start", "$native\n$page") else retry()
        return@evaluateJavascript
      }
      screenLooksBlank { blank ->
        if (blank == true) {
          Diagnostics.stage("screen blank at ${elapsed()} ms")
          if (final) showReport("The app is running but nothing is being drawn", "$native\n$page\nScreen capture is one flat colour.")
          else retry()
        } else {
          Diagnostics.markHealthy()
          if (force) showReport("Diagnostics requested", "$native\n$page\nScreen blank: $blank")
        }
      }
    }
    main.postDelayed({
      if (answered) return@postDelayed
      Diagnostics.stage("page not answering at ${elapsed()} ms")
      if (final) showReport(
        "The WebView stopped responding",
        "No answer from the page's JavaScript within ${PROBE_MS / 1000} s. " +
          "Renderer unresponsive: $rendererHung. WebView url=${wv.url} progress=${wv.progress}%"
      ) else retry()
    }, PROBE_MS)
  }

  // blank screen check
  private fun screenLooksBlank(done: (Boolean?) -> Unit) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return done(null)
    val root = window.decorView
    if (root.width <= 0 || root.height <= 0) return done(null)
    val bmp = Bitmap.createBitmap(root.width / 4, root.height / 4, Bitmap.Config.ARGB_8888)
    try {
      PixelCopy.request(window, bmp, { result ->
        if (result != PixelCopy.SUCCESS) {
          bmp.recycle()
          return@request done(null)
        }
        var lo = 255
        var hi = 0
        var y = 0
        while (y < bmp.height) {
          var x = 0
          while (x < bmp.width) {
            val c = bmp.getPixel(x, y)
            val l = (Color.red(c) * 3 + Color.green(c) * 6 + Color.blue(c)) / 10
            if (l < lo) lo = l
            if (l > hi) hi = l
            x += 6
          }
          y += 6
        }
        bmp.recycle()
        done(hi - lo < 12)
      }, main)
    } catch (t: Throwable) {
      bmp.recycle()
      done(null)
    }
  }

  // report
  private fun showReport(title: String, detail: String) {
    if (report != null || isFinishing) return
    Diagnostics.stage("showing report: $title")
    val body = buildString {
      append(detail).append("\n\n")
      append(Diagnostics.environment(this@MainActivity))
      append("Net lock: ").append(if (networkLocked) "on" else "off").append('\n')
      append("Renderer unresponsive: ").append(rendererHung).append('\n')
      append("Since start: ").append(elapsed()).append(" ms\n\n")
      append(Diagnostics.recentLog())
    }
    val actions = mutableListOf<Pair<String, () -> Unit>>()
    if (networkLocked) actions += "Retry without the WebView network lock" to { retryUnlocked() }
    actions += "Dismiss" to { dismissReport() }

    val view = Diagnostics.reportView(this, title, body, actions)
    addContentView(view, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    report = view
  }

  private fun dismissReport() {
    val view = report ?: return
    (view.parent as? ViewGroup)?.removeView(view)
    report = null
  }

  private fun retryUnlocked() {
    dismissReport()
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE)) return
    ProxyController.getInstance().clearProxyOverride({ it.run() }) {
      main.post {
        networkLocked = false
        Diagnostics.stage("network lock lifted, reloading")
        webView?.reload()
        main.postDelayed({ checkStarted(final = true, force = false) }, FINAL_CHECK_MS)
      }
    }
  }

  private fun decode(raw: String?): String = try {
    (JSONTokener(raw ?: "null").nextValue() as? String) ?: "probe returned ${raw ?: "nothing"}"
  } catch (t: Throwable) {
    "probe returned $raw"
  }

  private fun elapsed() = SystemClock.uptimeMillis() - startedAt

  private companion object {
    const val FIRST_CHECK_MS = 3_000L
    const val FINAL_CHECK_MS = 10_000L
    const val PROBE_MS = 3_000L
    const val PROBE = "(function(){try{" +
      "var a=document.getElementById('app');" +
      "var s=document.documentElement.getAttribute('data-screen');" +
      "return (a&&a.childElementCount>0&&s&&s!=='loading'?'MOUNTED':'NOT-READY')" +
      "+' screen='+s" +
      "+' readyState='+document.readyState" +
      "+' href='+location.href" +
      "+' size='+innerWidth+'x'+innerHeight" +
      "+' problems='+JSON.stringify(window.__SF_PROBLEMS__||[])" +
      "+' ua='+navigator.userAgent;" +
      "}catch(e){return 'PROBE-ERROR '+e}})()"
  }
}
