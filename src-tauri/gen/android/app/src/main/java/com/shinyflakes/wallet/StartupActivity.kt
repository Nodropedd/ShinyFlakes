// Launcher preflight
package com.shinyflakes.wallet

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Gravity
import android.webkit.WebView
import android.widget.TextView

class StartupActivity : Activity() {
  private val main = Handler(Looper.getMainLooper())

  @Volatile private var stage = "starting"
  private var settled = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Diagnostics.stage("StartupActivity created")
    val bg = Color.parseColor("#14161b")
    window.statusBarColor = bg
    window.navigationBarColor = bg

    val force = intent?.getBooleanExtra(Diagnostics.FORCE_EXTRA, false) == true

    // last start failed?
    val previous = if (force) null else Diagnostics.previousRunReport(this)
    if (previous != null) {
      Diagnostics.stage(Diagnostics.REPORT_SHOWN)
      setContentView(
        Diagnostics.reportView(
          this,
          "ShinyFlakes didn't start last time",
          previous,
          listOf("Continue" to { start(force) }),
        )
      )
      return
    }
    start(force)
  }

  private fun start(force: Boolean) {
    setContentView(TextView(this).apply {
      text = "Starting ShinyFlakes…"
      setTextColor(Color.parseColor("#939aa8"))
      textSize = 14f
      gravity = Gravity.CENTER
      setBackgroundColor(Color.parseColor("#14161b"))
    })

    // hang guard
    main.postDelayed({
      if (settled || isFinishing) return@postDelayed
      settled = true
      showFailure(IllegalStateException("Start-up is stuck while $stage. It did not finish within ${HANG_MS / 1000} s."))
    }, HANG_MS)

    Thread {
      val failure = preflight()
      main.post {
        if (settled || isFinishing) return@post
        settled = true
        if (failure == null) {
          Diagnostics.stage("handing over to the wallet")
          startActivity(Intent(this, MainActivity::class.java).putExtra(Diagnostics.FORCE_EXTRA, force))
          @Suppress("DEPRECATION")
          overridePendingTransition(0, 0)
          finish()
        } else {
          Log.e(Diagnostics.TAG, "start-up check failed", failure)
          showFailure(failure)
        }
      }
    }.start()
  }

  private fun preflight(): Throwable? {
    try {
      stage = "loading the wallet core (libshinyflakes_lib.so)"
      Diagnostics.stage(stage)
      System.loadLibrary("shinyflakes_lib")
    } catch (t: Throwable) {
      return t
    }
    try {
      stage = "checking the system WebView"
      Diagnostics.stage(stage)
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
    stage = "done"
    Diagnostics.stage("preflight passed")
    return null
  }

  private fun showFailure(t: Throwable) {
    Diagnostics.stage("start-up failed: ${t.javaClass.simpleName}")
    val body = buildString {
      append(Diagnostics.environment(this@StartupActivity)).append('\n')
      append(Log.getStackTraceString(t).ifBlank { t.toString() }).append("\n\n")
      append(Diagnostics.recentLog())
    }
    setContentView(
      Diagnostics.reportView(this, "ShinyFlakes could not start", body, emptyList())
    )
  }

  private companion object {
    const val HANG_MS = 12_000L
  }
}
