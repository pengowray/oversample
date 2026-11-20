package com.oversample.app

import android.accounts.Account
import android.app.Activity
import android.content.Context
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.auth.api.identity.RevokeAccessRequest
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.android.gms.common.api.Scope

private const val TAG = "GoogleAuthPlugin"
private const val DRIVE_FILE_SCOPE = "https://www.googleapis.com/auth/drive.file"
private const val PREFS = "google_auth"
private const val PREF_SIGNED_IN = "signed_in"
private const val PREF_ACCOUNT = "account"

/** Google's usual access-token lifetime; the authorization result does not give one. */
private const val TOKEN_LIFETIME_S = 3600

@InvokeArg
class AccessTokenArgs {
    var force: Boolean = false
}

@InvokeArg
class SetAccountArgs {
    var email: String = ""
}

/**
 * Google sign-in for the Drive copies of annotations and projects, through
 * Google Play services' AuthorizationClient (scope `drive.file`).
 *
 * Google finds this app's Android OAuth client from the package name and the
 * signing certificate, so no client id is in the app. There is no refresh
 * token: once the user has allowed access, `authorize()` hands out a new
 * access token without showing anything. The app keeps only a flag that the
 * user signed in and the account's email, so that a phone with several
 * Google accounts asks the right one. Google's reply does not name the
 * account when only `drive.file` is asked for (the call that would is
 * deprecated), so the webview passes the email Drive reports (`setAccount`).
 *
 * Replies match the desktop commands (`cmd_google.rs`). Errors are resolved
 * as `{ "error": code }` with the codes "unavailable", "cancelled",
 * "denied", "signed_out" and "failed".
 */
@TauriPlugin
class GoogleAuthPlugin(private val activity: Activity) : Plugin(activity) {

    private val prefs = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val client by lazy { Identity.getAuthorizationClient(activity) }

    /** The sign-in waiting for Google's account and consent screen. */
    private var pendingSignIn: Invoke? = null
    private var launcher: ActivityResultLauncher<IntentSenderRequest>? = null
    private var launchCount = 0

    /** The last token handed out, so that a forced renewal can clear it. */
    private var lastToken: String? = null

    private fun available(): Boolean =
        GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(activity) ==
            ConnectionResult.SUCCESS

    private fun signedIn(): Boolean = prefs.getBoolean(PREF_SIGNED_IN, false)

    private fun account(): Account? =
        prefs.getString(PREF_ACCOUNT, null)?.let { Account(it, "com.google") }

    private fun request(account: Account?, selectAccount: Boolean): AuthorizationRequest {
        val b = AuthorizationRequest.builder()
            .setRequestedScopes(listOf(Scope(DRIVE_FILE_SCOPE)))
        if (account != null) b.setAccount(account)
        if (selectAccount) b.setPrompt(AuthorizationRequest.Prompt.SELECT_ACCOUNT)
        return b.build()
    }

    private fun status(signedIn: Boolean): JSObject {
        val r = JSObject()
        r.put("available", true)
        r.put("signedIn", signedIn)
        r.put("persistent", true)
        return r
    }

    private fun error(invoke: Invoke, code: String) {
        val r = JSObject()
        r.put("error", code)
        invoke.resolve(r)
    }

    private fun tokenReply(token: String): JSObject {
        val r = JSObject()
        r.put("accessToken", token)
        r.put("expiresIn", TOKEN_LIFETIME_S)
        return r
    }

    /**
     * Signed in: remember it. The account's email arrives afterwards through
     * `setAccount`; a new sign-in may be another account, so the old one is
     * dropped.
     */
    private fun remember() {
        prefs.edit().putBoolean(PREF_SIGNED_IN, true).remove(PREF_ACCOUNT).apply()
    }

    private fun forget() {
        prefs.edit().clear().apply()
        lastToken = null
    }

    @Command
    fun status(invoke: Invoke) {
        if (!available()) {
            val r = JSObject()
            r.put("available", false)
            r.put("signedIn", false)
            r.put("persistent", false)
            invoke.resolve(r)
            return
        }
        invoke.resolve(status(signedIn()))
    }

    @Command
    fun signIn(invoke: Invoke) {
        if (!available()) return error(invoke, "unavailable")
        // A second press replaces a sign-in that is still waiting.
        pendingSignIn?.let { error(it, "cancelled") }
        pendingSignIn = invoke
        client.authorize(request(null, selectAccount = true))
            .addOnSuccessListener { result ->
                if (pendingSignIn !== invoke) return@addOnSuccessListener
                if (result.hasResolution()) {
                    val intent = result.pendingIntent
                    if (intent == null) {
                        finishSignIn(invoke, null, "failed")
                    } else {
                        launch(invoke, IntentSenderRequest.Builder(intent.intentSender).build())
                    }
                } else {
                    finishSignIn(invoke, result, null)
                }
            }
            .addOnFailureListener { e ->
                Log.w(TAG, "authorize failed", e)
                if (pendingSignIn === invoke) finishSignIn(invoke, null, "failed")
            }
    }

    /** Show Google's account and consent screen. */
    private fun launch(invoke: Invoke, req: IntentSenderRequest) {
        val host = activity as? ComponentActivity
        if (host == null) {
            finishSignIn(invoke, null, "failed")
            return
        }
        launcher?.unregister()
        launchCount += 1
        launcher = host.activityResultRegistry.register(
            "google-auth-sign-in-$launchCount",
            ActivityResultContracts.StartIntentSenderForResult()
        ) { activityResult ->
            launcher?.unregister()
            launcher = null
            if (pendingSignIn !== invoke) return@register
            if (activityResult.resultCode != Activity.RESULT_OK) {
                finishSignIn(invoke, null, "denied")
                return@register
            }
            try {
                val result = client.getAuthorizationResultFromIntent(activityResult.data)
                finishSignIn(invoke, result, null)
            } catch (e: Exception) {
                Log.w(TAG, "sign-in result unreadable", e)
                finishSignIn(invoke, null, "failed")
            }
        }
        launcher?.launch(req)
    }

    private fun finishSignIn(invoke: Invoke, result: AuthorizationResult?, code: String?) {
        if (pendingSignIn === invoke) pendingSignIn = null
        val token = result?.accessToken
        if (code != null || token == null) {
            error(invoke, code ?: "failed")
            return
        }
        remember()
        lastToken = token
        invoke.resolve(status(true))
    }

    @Command
    fun cancelSignIn(invoke: Invoke) {
        pendingSignIn?.let { error(it, "cancelled") }
        pendingSignIn = null
        invoke.resolve()
    }

    @Command
    fun accessToken(invoke: Invoke) {
        if (!available()) return error(invoke, "unavailable")
        if (!signedIn()) return error(invoke, "signed_out")
        val force = try {
            invoke.parseArgs(AccessTokenArgs::class.java).force
        } catch (e: Exception) {
            false
        }
        val old = lastToken
        if (force && old != null) {
            // Drive refused it: drop it from Google's cache first.
            client.clearToken(ClearTokenRequest.builder().setToken(old).build())
                .addOnCompleteListener { silentToken(invoke) }
        } else {
            silentToken(invoke)
        }
    }

    /** A token without showing anything; access that needs the screen again is signed out. */
    private fun silentToken(invoke: Invoke) {
        client.authorize(request(account(), selectAccount = false))
            .addOnSuccessListener { result ->
                val token = result.accessToken
                if (result.hasResolution() || token == null) {
                    Log.i(TAG, "access needs the consent screen again; signed out")
                    forget()
                    error(invoke, "signed_out")
                } else {
                    lastToken = token
                    invoke.resolve(tokenReply(token))
                }
            }
            .addOnFailureListener { e ->
                Log.w(TAG, "token renewal failed", e)
                error(invoke, "failed")
            }
    }

    /** The signed-in account's email, as Drive reports it. */
    @Command
    fun setAccount(invoke: Invoke) {
        val email = try {
            invoke.parseArgs(SetAccountArgs::class.java).email
        } catch (e: Exception) {
            ""
        }
        if (email.isNotEmpty() && signedIn()) {
            prefs.edit().putString(PREF_ACCOUNT, email).apply()
        }
        invoke.resolve()
    }

    @Command
    fun signOut(invoke: Invoke) {
        pendingSignIn?.let { error(it, "cancelled") }
        pendingSignIn = null
        val account = account()
        val token = lastToken
        forget()
        if (!available()) {
            invoke.resolve()
            return
        }
        token?.let { client.clearToken(ClearTokenRequest.builder().setToken(it).build()) }
        val revoke = RevokeAccessRequest.builder()
            .setScopes(listOf(Scope(DRIVE_FILE_SCOPE)))
        if (account != null) revoke.setAccount(account)
        try {
            client.revokeAccess(revoke.build())
                .addOnFailureListener { e -> Log.w(TAG, "revoke failed", e) }
                .addOnCompleteListener { invoke.resolve() }
        } catch (e: Exception) {
            // revokeAccess wants an account; without one the grant stays until the user removes it.
            Log.w(TAG, "revoke not possible", e)
            invoke.resolve()
        }
    }
}
