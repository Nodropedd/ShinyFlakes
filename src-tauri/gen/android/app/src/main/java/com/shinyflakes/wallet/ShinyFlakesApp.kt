// Process start
package com.shinyflakes.wallet

import android.app.Application
import android.content.Context

class ShinyFlakesApp : Application() {
  override fun attachBaseContext(base: Context) {
    super.attachBaseContext(base)
    Diagnostics.beginRun(base)
  }
}
