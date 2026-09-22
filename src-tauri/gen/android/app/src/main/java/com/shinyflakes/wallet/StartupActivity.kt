package com.shinyflakes.wallet

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.webkit.WebView
import android.widget.ScrollView
import android.widget.TextView

/**
 * The launcher. Its only job is to prove the app *can* start before handing
 * over to the Tauri activity.
 *
 * Everything that makes this app go — the Rust core and the system WebView —
 * is loaded inside `WryActivity.onCreate`, and any failure there kills the
 * activity with the window still showing the theme background. The user sees
 * a black screen and has nothing to report; worse, the same failure then
 * re-fires from `onWindowFocusChanged` and the other lifecycle callbacks
 * `WryActivity` overrides, so there is no safe way to catch it once that
 * activity has started.
 *
 * So check first, from a plain activity that owns none of that machinery. If
 * the check passes, the real activity starts normally and this one gets out
 * of the way. If it fails, the reason goes on the screen instead of a black
 * rectangle.
 *
 * `System.loadLibrary` is idempotent within a process, so loading the library
 * here is not extra work — it is the same load `Rust`'s class initialiser
 * would do a moment later, just somewhere we can catch it.
 */
class StartupActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)

    val failure = preflight()
    if (failure == null) {
      startActivity(Intent(this, MainActivity::class.java))
      // No transition, so a healthy start looks like one launch, not two.
      overridePendingTransition(0, 0)
      finish()
      return
    }

    Log.e(TAG, "start-up check failed", failure)
    showFailure(failure)
  }

  /** Returns null when the app can start, or the reason it cannot. */
  private fun preflight(): Throwable? {
    try {
      System.loadLibrary("shinyflakes_lib")
    } catch (t: Throwable) {
      return t
    }
    try {
      // Touching the provider surfaces a WebView that is absent, disabled or
      // failing to initialise, which on a vendor ROM is the other way this
      // app never draws anything.
      if (WebView.getCurrentWebViewPackage() == null) {
        return IllegalStateException(
          "No Android System WebView is installed or enabled on this device. " +
            "ShinyFlakes draws its whole interface in a WebView, so it cannot " +
            "start without one. Enable or install 'Android System WebView' " +
            "(or Chrome, on some devices) and try again."
        )
      }
    } catch (t: Throwable) {
      return t
    }
    return null
  }

  private fun showFailure(t: Throwable) {
    val report = buildString {
      append("ShinyFlakes could not start on this device.\n\n")
      append("Please send this text — it says exactly what failed.\n\n")
      append("Device:  ${Build.MANUFACTURER} ${Build.MODEL}\n")
      append("Android: ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})\n")
      append("ABIs:    ${Build.SUPPORTED_ABIS.joinToString(", ")}\n")
      append("WebView: ${webViewVersion()}\n\n")
      append(Log.getStackTraceString(t).ifBlank { t.toString() })
    }

    // The default theme paints a coloured status bar; the wallet's own
    // activity hides it behind edge-to-edge, so match it here rather than
    // topping the error with an unrelated purple band.
    window.statusBarColor = Color.parseColor("#14161b")
    window.navigationBarColor = Color.parseColor("#14161b")

    val text = TextView(this).apply {
      setTextColor(Color.parseColor("#e8eaef"))
      textSize = 12f
      setPadding(48, 120, 48, 64)
      setTextIsSelectable(true)
      this.text = report
    }
    setContentView(
      ScrollView(this).apply {
        setBackgroundColor(Color.parseColor("#14161b"))
        addView(text)
      }
    )
  }

  private fun webViewVersion(): String =
    try {
      WebView.getCurrentWebViewPackage()
        ?.let { "${it.packageName} ${it.versionName}" }
        ?: "none installed or enabled"
    } catch (t: Throwable) {
      "unavailable (${t.javaClass.simpleName}: ${t.message})"
    }

  private companion object {
    const val TAG = "ShinyFlakes"
  }
}
