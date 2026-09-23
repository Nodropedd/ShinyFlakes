// Start-up diagnostics
package com.shinyflakes.wallet

import android.app.Activity
import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.system.Os
import android.system.OsConstants
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

object Diagnostics {
  const val FORCE_EXTRA = "sf_diagnostics"
  const val TAG = "ShinyFlakes"

  private val BG = Color.parseColor("#14161b")
  private val TEXT = Color.parseColor("#e8eaef")
  private val GOLD = Color.parseColor("#ffc83d")

  private val NOISE = listOf(
    "s_glBindAttribLocation", "ImeTracker", "InputMethod", "ProfileInstaller",
    "--------- beginning of",
  )

  // environment
  fun environment(context: Context): String = buildString {
    append("App:      ").append(appVersion(context)).append('\n')
    append("Device:   ").append(Build.MANUFACTURER).append(' ').append(Build.MODEL).append('\n')
    append("Android:  ").append(Build.VERSION.RELEASE).append(" (API ").append(Build.VERSION.SDK_INT).append(")\n")
    append("ABIs:     ").append(Build.SUPPORTED_ABIS.joinToString(", ")).append('\n')
    append("Running:  ").append(System.getProperty("os.arch")).append(", libs in ")
      .append(context.applicationInfo.nativeLibraryDir).append('\n')
    append("Page:     ").append(pageSize()).append(" bytes\n")
    append("WebView:  ").append(webView()).append('\n')
  }

  // own log
  fun recentLog(maxLines: Int = 120): String = try {
    val pid = android.os.Process.myPid().toString()
    val proc = ProcessBuilder("logcat", "-d", "-v", "brief", "--pid", pid, "-t", "800")
      .redirectErrorStream(true)
      .start()
    val lines = proc.inputStream.bufferedReader().readLines()
    proc.waitFor()
    val kept = lines.filter { line -> NOISE.none { line.contains(it) } }.takeLast(maxLines)
    "Recent log (${kept.size} lines):\n" + kept.joinToString("\n")
  } catch (t: Throwable) {
    "Recent log unavailable: $t"
  }

  // report view
  fun reportView(
    activity: Activity,
    title: String,
    body: String,
    actions: List<Pair<String, () -> Unit>>,
  ): View {
    val pad = (16 * activity.resources.displayMetrics.density).toInt()
    val column = LinearLayout(activity).apply {
      orientation = LinearLayout.VERTICAL
      setPadding(pad, pad * 4, pad, pad * 2)
      setBackgroundColor(BG)
    }
    column.addView(TextView(activity).apply {
      text = title
      setTextColor(GOLD)
      textSize = 17f
      typeface = Typeface.DEFAULT_BOLD
    })
    column.addView(TextView(activity).apply {
      text = "Take a screenshot of this screen and send it. It says exactly what failed."
      setTextColor(TEXT)
      textSize = 13f
      setPadding(0, pad / 2, 0, pad)
    })
    for ((label, action) in actions) {
      column.addView(Button(activity).apply {
        text = label
        isAllCaps = false
        setOnClickListener { action() }
      })
    }
    column.addView(TextView(activity).apply {
      text = body
      setTextColor(TEXT)
      textSize = 10.5f
      typeface = Typeface.MONOSPACE
      setTextIsSelectable(true)
      setPadding(0, pad, 0, 0)
    })
    return ScrollView(activity).apply {
      setBackgroundColor(BG)
      isFillViewport = true
      addView(column, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
    }
  }

  private fun appVersion(context: Context): String = try {
    @Suppress("DEPRECATION")
    val info = context.packageManager.getPackageInfo(context.packageName, 0)
    @Suppress("DEPRECATION")
    "${info.versionName} (${info.versionCode})"
  } catch (t: Throwable) {
    "unknown"
  }

  private fun pageSize(): Long = try {
    Os.sysconf(OsConstants._SC_PAGESIZE)
  } catch (t: Throwable) {
    -1
  }

  private fun webView(): String = try {
    WebView.getCurrentWebViewPackage()
      ?.let { "${it.packageName} ${it.versionName}" }
      ?: "none installed or enabled"
  } catch (t: Throwable) {
    "unavailable (${t.javaClass.simpleName}: ${t.message})"
  }
}
