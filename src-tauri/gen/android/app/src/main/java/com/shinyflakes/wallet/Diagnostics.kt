// Start-up diagnostics
package com.shinyflakes.wallet

import android.app.Activity
import android.app.ActivityManager
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.system.Os
import android.system.OsConstants
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import java.io.File
import java.util.Date
import java.util.concurrent.atomic.AtomicLong

object Diagnostics {
  const val FORCE_EXTRA = "sf_diagnostics"
  const val TAG = "ShinyFlakes"

  private const val LOG = "startup.log"
  private const val PREV_LOG = "startup-previous.log"
  private const val HANG = "startup-hang.txt"
  private const val PREV_HANG = "startup-hang-previous.txt"
  private const val TEST_HANG = "test-hang"
  private const val HEALTHY = "healthy"
  const val REPORT_SHOWN = "showing previous-start report"
  private const val FREEZE_KILL_MS = 10_000L

  private val BG = Color.parseColor("#14161b")
  private val TEXT = Color.parseColor("#e8eaef")
  private val GOLD = Color.parseColor("#ffc83d")

  private val NOISE = listOf(
    "s_glBindAttribLocation", "ImeTracker", "InputMethod", "ProfileInstaller",
    "--------- beginning of",
  )

  @Volatile private var dir: File? = null
  @Volatile private var app: Context? = null
  @Volatile var launcherRan = false
  @Volatile private var runStart = 0L
  @Volatile private var lastStage = "starting"
  @Volatile private var healthy = false

  // run log
  fun beginRun(context: Context) {
    runStart = SystemClock.uptimeMillis()
    app = context
    val d = context.filesDir ?: return
    dir = d
    rotate(File(d, LOG), File(d, PREV_LOG))
    rotate(File(d, HANG), File(d, PREV_HANG))
    stage("process started")
    watchMainThread()
  }

  fun stage(what: String) {
    lastStage = what
    val d = dir ?: return
    try {
      File(d, LOG).appendText("+${SystemClock.uptimeMillis() - runStart} ms  $what\n")
    } catch (_: Throwable) {
    }
  }

  fun markHealthy() {
    if (healthy) return
    healthy = true
    stage(HEALTHY)
  }

  // test hook, file only adb can plant
  fun takeTestHang(): Boolean {
    val f = File(dir ?: return false, TEST_HANG)
    return f.exists() && f.delete()
  }

  private fun rotate(current: File, previous: File) {
    try {
      previous.delete()
      if (current.exists()) current.renameTo(previous)
    } catch (_: Throwable) {
    }
  }

  // freeze watchdog
  private fun watchMainThread() {
    val main = Handler(Looper.getMainLooper())
    val mainThread = Looper.getMainLooper().thread
    val beat = AtomicLong(SystemClock.uptimeMillis())
    Thread({
      val until = SystemClock.uptimeMillis() + 90_000
      while (!healthy && SystemClock.uptimeMillis() < until) {
        main.post { beat.set(SystemClock.uptimeMillis()) }
        try {
          Thread.sleep(1_000)
        } catch (_: InterruptedException) {
          return@Thread
        }
        val stuck = SystemClock.uptimeMillis() - beat.get()
        if (stuck > 4_000) writeHang(stuck, mainThread.stackTrace)
        // frozen start: close, so the next open reports it
        if (stuck > FREEZE_KILL_MS) {
          stage("closed itself after ${stuck / 1000} s frozen")
          // drop the task, so reopening starts at the launcher
          try {
            app?.getSystemService(ActivityManager::class.java)?.appTasks?.forEach { it.finishAndRemoveTask() }
          } catch (_: Throwable) {
          }
          android.os.Process.killProcess(android.os.Process.myPid())
        }
      }
    }, "sf-freeze-watch").apply { isDaemon = true }.start()
  }

  private fun writeHang(stuckMs: Long, stack: Array<StackTraceElement>) {
    val d = dir ?: return
    try {
      File(d, HANG).writeText(buildString {
        append("Main thread frozen for ").append(stuckMs).append(" ms, ")
        append("+").append(SystemClock.uptimeMillis() - runStart).append(" ms after start, ")
        append("during: ").append(lastStage).append('\n')
        stack.take(40).forEach { append("  at ").append(it).append('\n') }
      })
    } catch (_: Throwable) {
    }
  }

  // previous start
  fun previousRunReport(context: Context): String? {
    val d = context.filesDir ?: return null
    val log = File(d, PREV_LOG).takeIf { it.exists() }?.readText() ?: return null
    if (log.contains(HEALTHY)) return null
    if (!log.contains("StartupActivity") && !log.contains("MainActivity")) return null
    // only showed this report
    if (log.trim().lines().last().substringAfter("ms  ") == REPORT_SHOWN) return null
    val hang = File(d, PREV_HANG).takeIf { it.exists() }?.readText()
    return buildString {
      append("How far it got:\n").append(log).append('\n')
      hang?.let { append(it).append('\n') }
      append(exitRecord(context)).append('\n')
      append(environment(context))
    }
  }

  private fun exitRecord(context: Context): String {
    if (Build.VERSION.SDK_INT < 30) return "Android exit record: needs Android 11+"
    return try {
      val am = context.getSystemService(ActivityManager::class.java)
      val last = am.getHistoricalProcessExitReasons(context.packageName, 0, 8)
        .firstOrNull { it.processName == context.packageName }
        ?: return "Android exit record: none"
      buildString {
        append("Android says it ended: ").append(reasonName(last.reason))
        append(" (status ").append(last.status).append(")\n")
        last.description?.let { append("Detail: ").append(it).append('\n') }
        append("When: ").append(Date(last.timestamp)).append('\n')
        val trace = try {
          last.traceInputStream?.use { it.readBytes() }
        } catch (_: Throwable) {
          null
        }
        if (trace != null && trace.isNotEmpty()) append("Trace:\n").append(readable(trace)).append('\n')
      }
    } catch (t: Throwable) {
      "Android exit record unavailable: $t"
    }
  }

  private fun reasonName(r: Int) = when (r) {
    1 -> "exited itself"
    2 -> "killed by a signal"
    3 -> "low memory"
    4 -> "crash (Java)"
    5 -> "crash (native)"
    6 -> "not responding (ANR)"
    7 -> "failed to initialise"
    8 -> "permission change"
    9 -> "too much resource use"
    10 -> "closed by the user"
    11 -> "force-stopped by the user"
    12 -> "a process it depended on died"
    13 -> "killed by the system"
    14 -> "frozen by the system"
    15, 16 -> "app updated or changed"
    else -> "unknown ($r)"
  }

  // ANR text or native tombstone
  private fun readable(bytes: ByteArray): String {
    val text = String(bytes, Charsets.ISO_8859_1)
    val main = text.indexOf("\"main\"")
    if (main >= 0) return text.substring(main).lineSequence().take(45).joinToString("\n")
    return text.split(Regex("[^\\x20-\\x7e]+"))
      .map { it.trim() }
      .filter { it.length >= 5 }
      .distinct()
      .take(90)
      .joinToString("\n")
  }

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
      text = "Tap Copy report and send the text. It says exactly what failed."
      setTextColor(TEXT)
      textSize = 13f
      setPadding(0, pad / 2, 0, pad)
    })
    val copy = "Copy report" to {
      val clip = ClipData.newPlainText("ShinyFlakes report", "$title\n\n$body")
      activity.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(clip)
      Toast.makeText(activity, "Report copied", Toast.LENGTH_SHORT).show()
    }
    for ((label, action) in listOf(copy) + actions) {
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

  fun versionName(context: Context): String = try {
    context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""
  } catch (t: Throwable) {
    ""
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
