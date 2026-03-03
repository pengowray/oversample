package com.batmonic.app

import android.os.Bundle
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private var usbAudioPlugin: UsbAudioPlugin? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // Register USB audio plugin before super.onCreate (which initializes the WebView)
    val plugin = UsbAudioPlugin(this)
    usbAudioPlugin = plugin
    pluginManager.load(null, "usb-audio", plugin, "{}")
    super.onCreate(savedInstanceState)

    // Override WebChromeClient to auto-grant audio capture permission to the WebView.
    // This is needed for Browser mic mode to work on Android (getUserMedia).
    setupWebViewPermissions()
  }

  override fun onRequestPermissionsResult(
    requestCode: Int,
    permissions: Array<out String>,
    grantResults: IntArray
  ) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    // Forward to USB audio plugin for RECORD_AUDIO permission handling
    usbAudioPlugin?.handlePermissionResult(requestCode, grantResults)
  }

  private fun setupWebViewPermissions() {
    // Find the WebView created by Tauri and override its WebChromeClient
    // to grant RESOURCE_AUDIO_CAPTURE requests from the frontend JavaScript.
    val rootView = window.decorView
    findWebView(rootView)?.let { webView ->
      val originalClient = webView.webChromeClient
      webView.webChromeClient = object : WebChromeClient() {
        override fun onPermissionRequest(request: PermissionRequest) {
          val resources = request.resources
          if (resources.contains(PermissionRequest.RESOURCE_AUDIO_CAPTURE)) {
            request.grant(resources)
          } else {
            originalClient?.onPermissionRequest(request) ?: request.deny()
          }
        }
      }
    }
  }

  private fun findWebView(view: android.view.View): WebView? {
    if (view is WebView) return view
    if (view is android.view.ViewGroup) {
      for (i in 0 until view.childCount) {
        findWebView(view.getChildAt(i))?.let { return it }
      }
    }
    return null
  }
}
