package com.oversample.app

import android.app.Activity
import android.util.Log
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

private const val TAG = "ZoomPlugin"

/**
 * Reset the WebView's pinch-zoom level to 100%.
 *
 * There is no JavaScript-side API to reset visualViewport.scale — the meta
 * viewport tag is only consulted at page load in Android WebView, and
 * documentElement.style.zoom doesn't affect the visual viewport. The only
 * reliable mechanism is the native WebView.zoomBy() method on the UI
 * thread. zoomBy(0.01f) clamps to the WebView's minimum scale, which with
 * the default settings (setSupportZoom=true) is 100%.
 */
@TauriPlugin
class ZoomPlugin(private val activity: Activity) : Plugin(activity) {

    private var webView: WebView? = null

    fun setWebView(wv: WebView) {
        webView = wv
        Log.i(TAG, "WebView attached")
    }

    @Command
    fun reset(invoke: Invoke) {
        val wv = webView
        if (wv == null) {
            Log.w(TAG, "reset() called before WebView attached")
            invoke.reject("WebView not initialized")
            return
        }
        activity.runOnUiThread {
            // 0.01 is the minimum allowed zoomFactor; the WebView clamps it
            // to its built-in minimum scale (default 1.0 = 100%).
            wv.zoomBy(0.01f)
        }
        invoke.resolve()
    }
}
