package com.oversample.app

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.Executors
import java.util.function.Consumer

private const val TAG = "GeolocationPlugin"
private const val LOCATION_PERMISSION_REQUEST_CODE = 9010
private const val LOCATION_TIMEOUT_MS = 10_000L

@TauriPlugin
class GeolocationPlugin(private val activity: Activity) : Plugin(activity) {

    private var pendingPermissionInvoke: Invoke? = null
    private val handler = Handler(Looper.getMainLooper())

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)
        Log.i(TAG, "GeolocationPlugin loaded")
    }

    @Command
    fun getCurrentLocation(invoke: Invoke) {
        if (!hasLocationPermission()) {
            pendingPermissionInvoke = invoke
            requestLocationPermission()
            return
        }
        doGetCurrentLocation(invoke)
    }

    fun handlePermissionResult(requestCode: Int, grantResults: IntArray) {
        if (requestCode != LOCATION_PERMISSION_REQUEST_CODE) return
        val invoke = pendingPermissionInvoke ?: return
        pendingPermissionInvoke = null

        if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            doGetCurrentLocation(invoke)
        } else {
            val result = JSObject()
            result.put("error", "permission_denied")
            invoke.resolve(result)
        }
    }

    private fun hasLocationPermission(): Boolean {
        return ContextCompat.checkSelfPermission(activity, Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED
    }

    private fun requestLocationPermission() {
        ActivityCompat.requestPermissions(
            activity,
            arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION),
            LOCATION_PERMISSION_REQUEST_CODE
        )
    }

    private fun doGetCurrentLocation(invoke: Invoke) {
        val locationManager = activity.getSystemService(Context.LOCATION_SERVICE) as? LocationManager
        if (locationManager == null) {
            val result = JSObject()
            result.put("error", "no_location_service")
            invoke.resolve(result)
            return
        }

        // Check if any provider is enabled
        val hasGps = locationManager.isProviderEnabled(LocationManager.GPS_PROVIDER)
        val hasNetwork = locationManager.isProviderEnabled(LocationManager.NETWORK_PROVIDER)
        if (!hasGps && !hasNetwork) {
            val result = JSObject()
            result.put("error", "location_disabled")
            invoke.resolve(result)
            return
        }

        // Try last known location first (fast path)
        try {
            val lastGps = locationManager.getLastKnownLocation(LocationManager.GPS_PROVIDER)
            val lastNet = locationManager.getLastKnownLocation(LocationManager.NETWORK_PROVIDER)
            val best = pickBestLocation(lastGps, lastNet)
            if (best != null && best.time > System.currentTimeMillis() - 120_000) {
                // Recent enough (< 2 minutes old)
                invoke.resolve(locationToJSObject(best))
                return
            }
        } catch (e: SecurityException) {
            Log.w(TAG, "SecurityException checking last known location", e)
        }

        // Request a fresh location fix
        val provider = if (hasGps) LocationManager.GPS_PROVIDER else LocationManager.NETWORK_PROVIDER

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            // API 30+: use getCurrentLocation with CancellationSignal
            try {
                val cancellationSignal = CancellationSignal()
                handler.postDelayed({
                    cancellationSignal.cancel()
                }, LOCATION_TIMEOUT_MS)

                val executor = Executors.newSingleThreadExecutor()
                locationManager.getCurrentLocation(
                    provider,
                    cancellationSignal,
                    executor,
                    Consumer<Location?> { location ->
                        if (location != null) {
                            invoke.resolve(locationToJSObject(location))
                        } else {
                            // getCurrentLocation returned null — try last known as fallback
                            val fallback = try {
                                locationManager.getLastKnownLocation(LocationManager.GPS_PROVIDER)
                                    ?: locationManager.getLastKnownLocation(LocationManager.NETWORK_PROVIDER)
                            } catch (e: SecurityException) { null }

                            if (fallback != null) {
                                invoke.resolve(locationToJSObject(fallback))
                            } else {
                                val result = JSObject()
                                result.put("error", "timeout")
                                invoke.resolve(result)
                            }
                        }
                    }
                )
            } catch (e: SecurityException) {
                val result = JSObject()
                result.put("error", "permission_denied")
                invoke.resolve(result)
            }
        } else {
            // API 26-29: use requestSingleUpdate
            try {
                var resolved = false
                val listener = object : LocationListener {
                    override fun onLocationChanged(location: Location) {
                        if (!resolved) {
                            resolved = true
                            locationManager.removeUpdates(this)
                            invoke.resolve(locationToJSObject(location))
                        }
                    }
                    @Deprecated("Deprecated in API")
                    override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
                    override fun onProviderEnabled(provider: String) {}
                    override fun onProviderDisabled(provider: String) {}
                }

                locationManager.requestSingleUpdate(provider, listener, Looper.getMainLooper())

                // Timeout fallback
                handler.postDelayed({
                    if (!resolved) {
                        resolved = true
                        locationManager.removeUpdates(listener)
                        val fallback = try {
                            locationManager.getLastKnownLocation(LocationManager.GPS_PROVIDER)
                                ?: locationManager.getLastKnownLocation(LocationManager.NETWORK_PROVIDER)
                        } catch (e: SecurityException) { null }

                        if (fallback != null) {
                            invoke.resolve(locationToJSObject(fallback))
                        } else {
                            val result = JSObject()
                            result.put("error", "timeout")
                            invoke.resolve(result)
                        }
                    }
                }, LOCATION_TIMEOUT_MS)
            } catch (e: SecurityException) {
                val result = JSObject()
                result.put("error", "permission_denied")
                invoke.resolve(result)
            }
        }
    }

    private fun pickBestLocation(a: Location?, b: Location?): Location? {
        if (a == null) return b
        if (b == null) return a
        // Prefer more recent, then more accurate
        return if (a.time >= b.time - 30_000 && a.accuracy <= b.accuracy) a else b
    }

    private fun locationToJSObject(location: Location): JSObject {
        val result = JSObject()
        result.put("latitude", location.latitude)
        result.put("longitude", location.longitude)
        result.put("accuracy", location.accuracy.toDouble())
        result.put("hasAltitude", location.hasAltitude())
        if (location.hasAltitude()) {
            result.put("altitude", location.altitude)
        }
        return result
    }
}
