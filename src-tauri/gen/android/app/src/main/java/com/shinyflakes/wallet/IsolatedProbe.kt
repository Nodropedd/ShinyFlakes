// Isolated process test
package com.shinyflakes.wallet

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder

class IsolatedProbe : Service() {
  override fun onBind(intent: Intent?): IBinder = Binder()
}
