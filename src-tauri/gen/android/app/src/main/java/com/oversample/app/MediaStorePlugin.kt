package com.oversample.app

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.ContentValues
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

private const val TAG = "MediaStorePlugin"
private const val SUBFOLDER = "Oversample"
private const val WRITE_STORAGE_REQUEST_CODE = 9001

@InvokeArg
data class CreateRecordingEntryArgs(val filename: String = "")

@InvokeArg
data class OpenForReadArgs(val uri: String = "")

@InvokeArg
data class CreateExportEntryArgs(
    val filename: String = "",
    val mimeType: String = "",
    val relativePath: String = "",
)

@InvokeArg
data class ExportEntryArgs(val uri: String = "", val mimeType: String = "")

@TauriPlugin
class MediaStorePlugin(private val activity: Activity) : Plugin(activity) {

    private var pendingPermissionInvoke: Invoke? = null

    // For fd-passing recording: Kotlin holds the MediaStore entry open
    private var pendingRecordingUri: Uri? = null

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)
        Log.i(TAG, "MediaStorePlugin loaded")
    }

    // ── Exports: fd-passing, like recordings ────────────────────────────

    /**
     * Create a shared-storage entry for an export and return a raw POSIX fd
     * that Rust writes the bytes to (`save_adopt_fd`). See [createEntry].
     * Returns no fd when the pre-Q storage permission was just asked for.
     */
    @Command
    fun createExportEntry(invoke: Invoke) {
        val args = invoke.parseArgs(CreateExportEntryArgs::class.java)
        try {
            val entry = createEntry(
                invoke,
                args.filename.ifEmpty { "export" },
                args.mimeType,
                args.relativePath.ifEmpty { "Download/$SUBFOLDER/exports" },
            ) ?: return
            val result = JSObject()
            result.put("fd", entry.fd)
            result.put("uri", entry.uri)
            result.put("path", entry.uri)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "createExportEntry failed", e)
            invoke.reject("Failed to create export entry: ${e.message}")
        }
    }

    /**
     * Make a finished export visible: IS_PENDING=0 on API 29+, a media scan on
     * older versions. Call after Rust has closed the fd.
     */
    @Command
    fun finishExportEntry(invoke: Invoke) {
        val args = invoke.parseArgs(ExportEntryArgs::class.java)
        try {
            if (args.uri.startsWith("content://")) {
                val values = ContentValues().apply {
                    put(MediaStore.MediaColumns.IS_PENDING, 0)
                }
                activity.contentResolver.update(Uri.parse(args.uri), values, null, null)
            } else {
                MediaScannerConnection.scanFile(
                    activity,
                    arrayOf(args.uri),
                    arrayOf(args.mimeType),
                    null
                )
            }
            Log.i(TAG, "Finished export entry: ${args.uri}")
            invoke.resolve(JSObject())
        } catch (e: Exception) {
            Log.e(TAG, "finishExportEntry failed", e)
            invoke.reject("Failed to finish export: ${e.message}")
        }
    }

    /** Delete an export entry whose bytes could not all be written. */
    @Command
    fun cancelExportEntry(invoke: Invoke) {
        val args = invoke.parseArgs(ExportEntryArgs::class.java)
        try {
            if (args.uri.startsWith("content://")) {
                activity.contentResolver.delete(Uri.parse(args.uri), null, null)
            } else {
                File(args.uri).delete()
            }
            Log.i(TAG, "Cancelled export entry: ${args.uri}")
            invoke.resolve(JSObject())
        } catch (e: Exception) {
            Log.e(TAG, "cancelExportEntry failed", e)
            invoke.reject("Failed to cancel export: ${e.message}")
        }
    }

    // ── ContentResolver fd-passing for direct recording ─────────────────

    /**
     * Create a shared-storage entry for a new recording in
     * Recordings/Oversample and return a raw POSIX fd that Rust writes the WAV
     * to directly. See [createEntry]. Call finalizeRecordingEntry when writing
     * is complete.
     */
    @Command
    fun createRecordingEntry(invoke: Invoke) {
        val args = invoke.parseArgs(CreateRecordingEntryArgs::class.java)
        try {
            val entry = createEntry(
                invoke,
                args.filename.ifEmpty { "recording.wav" },
                "audio/wav",
                "Recordings/$SUBFOLDER",
            ) ?: return
            pendingRecordingUri = entry.pendingUri
            val result = JSObject()
            result.put("fd", entry.fd)
            result.put("uri", entry.uri)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "createRecordingEntry failed", e)
            invoke.reject("Failed to create recording entry: ${e.message}")
        }
    }

    /**
     * Finalize a recording entry created by createRecordingEntry.
     * Sets IS_PENDING=0 on API 29+ so the file becomes visible.
     * On pre-Q, triggers a MediaScanner scan.
     */
    @Command
    fun finalizeRecordingEntry(invoke: Invoke) {
        try {
            val uri = pendingRecordingUri
            if (uri != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val resolver = activity.contentResolver
                val values = ContentValues().apply {
                    put(MediaStore.Audio.Media.IS_PENDING, 0)
                }
                resolver.update(uri, values, null, null)
                Log.i(TAG, "Finalized recording entry: $uri")
                pendingRecordingUri = null
            } else {
                // Pre-Q: scan via MediaScanner (the file path was returned as uri)
                // No-op needed here; the file is already visible
                Log.i(TAG, "Finalized recording entry (pre-Q, no-op)")
            }
            val result = JSObject()
            result.put("ok", true)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "finalizeRecordingEntry failed", e)
            invoke.reject("Failed to finalize: ${e.message}")
        }
    }

    /**
     * Delete our own pending (IS_PENDING=1) MediaStore entries in Oversample
     * folders: recordings and exports, in the audio, video and download
     * collections. Called once on app startup to clean up after a crash that
     * left rows unfinished. Without this, apps that ignore IS_PENDING would show
     * zero-byte or half-written files, and they would pile up.
     *
     * Safe: scoped storage on API 29+ restricts this query to entries our
     * own app owns, so we can't accidentally delete other apps' in-progress
     * writes. No-op on API < 29 (pre-Q has no IS_PENDING concept).
     */
    @Command
    fun cleanupPendingEntries(invoke: Invoke) {
        try {
            val result = JSObject()
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
                result.put("deleted", 0)
                result.put("skipped", true)
                invoke.resolve(result)
                return
            }

            val resolver = activity.contentResolver
            // Only look inside our own folders (Recordings/Oversample/,
            // Movies/Oversample/exports/, ...) so we don't touch anything else.
            val selection = "${MediaStore.MediaColumns.IS_PENDING} = 1 AND " +
                "${MediaStore.MediaColumns.RELATIVE_PATH} LIKE ?"
            val args = arrayOf("%/$SUBFOLDER/%")
            val projection = arrayOf(
                MediaStore.MediaColumns._ID,
                MediaStore.MediaColumns.DISPLAY_NAME,
            )
            val collections = listOf(
                MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
                MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
                MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
            )

            var deleted = 0
            val names = mutableListOf<String>()
            for (collection in collections) {
                resolver.query(collection, projection, selection, args, null)?.use { cursor ->
                    val idColumn = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                    val nameColumn = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
                    while (cursor.moveToNext()) {
                        val id = cursor.getLong(idColumn)
                        val name = cursor.getString(nameColumn) ?: "unknown"
                        val uri = ContentUris.withAppendedId(collection, id)
                        try {
                            val rows = resolver.delete(uri, null, null)
                            if (rows > 0) {
                                deleted++
                                names.add(name)
                            }
                        } catch (e: Exception) {
                            Log.w(TAG, "Failed to delete pending entry $uri: ${e.message}")
                        }
                    }
                }
            }

            if (deleted > 0) {
                Log.i(TAG, "Cleaned up $deleted pending MediaStore entries: $names")
            }
            result.put("deleted", deleted)
            result.put("skipped", false)
            // Also clear our own in-memory handle — its target may have been
            // one of the rows we just deleted.
            pendingRecordingUri = null
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "cleanupPendingEntries failed", e)
            invoke.reject("Failed to cleanup pending entries: ${e.message}")
        }
    }

    /**
     * Cancel a pending recording entry created by createRecordingEntry.
     * Deletes the MediaStore row so no orphaned IS_PENDING=1 entry remains.
     * Safe to call even if no entry is pending (no-op).
     */
    @Command
    fun cancelRecordingEntry(invoke: Invoke) {
        try {
            val uri = pendingRecordingUri
            if (uri != null) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    activity.contentResolver.delete(uri, null, null)
                    Log.i(TAG, "Cancelled pending recording entry: $uri")
                } else {
                    // Pre-Q: delete the file directly
                    val path = uri.path
                    if (path != null) {
                        val file = File(path)
                        if (file.exists()) file.delete()
                        Log.i(TAG, "Cancelled pending recording file: $path")
                    }
                }
                pendingRecordingUri = null
            } else {
                Log.i(TAG, "cancelRecordingEntry: no pending entry")
            }
            val result = JSObject()
            result.put("ok", true)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "cancelRecordingEntry failed", e)
            // Still clear the reference — best effort
            pendingRecordingUri = null
            invoke.reject("Failed to cancel: ${e.message}")
        }
    }

    /**
     * Open a recording's content:// URI for reading and return a raw POSIX fd.
     * Rust reads byte ranges from it (`read_fd_range`), so the bytes reach the
     * webview raw instead of as base64. The app keeps the fd open until it exits.
     */
    @Command
    fun openRecordingForRead(invoke: Invoke) {
        val args = invoke.parseArgs(OpenForReadArgs::class.java)
        try {
            val pfd = activity.contentResolver.openFileDescriptor(Uri.parse(args.uri), "r")
                ?: throw Exception("openFileDescriptor returned null for ${args.uri}")
            val result = JSObject()
            result.put("fd", pfd.detachFd()) // Rust owns the fd now
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "openRecordingForRead failed", e)
            invoke.reject("openRecordingForRead failed: ${e.message}")
        }
    }

    // ── Export file via SAF picker ───────────────────────────────────────

    // ── Permission handling ─────────────────────────────────────────────

    fun handlePermissionResult(requestCode: Int, grantResults: IntArray) {
        if (requestCode != WRITE_STORAGE_REQUEST_CODE) return
        val invoke = pendingPermissionInvoke ?: return
        pendingPermissionInvoke = null

        if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            // Retry the original command — re-invoke via the plugin command
            // For simplicity, just tell the frontend to retry
            val result = JSObject()
            result.put("permissionGranted", true)
            result.put("retry", true)
            invoke.resolve(result)
        } else {
            invoke.reject("Storage permission denied")
        }
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /** A new shared-storage entry and the fd that Rust writes it through. */
    private class NewEntry(
        val fd: Int,
        /** The content:// URI (API 29+) or the file path (older versions). */
        val uri: String,
        /** The MediaStore row (API 29+), to finish or delete later. */
        val pendingUri: Uri?,
    )

    /**
     * Create a shared-storage entry and open it for writing.
     *
     * - API 29+: a MediaStore row in the collection that matches [mimeType],
     *   in [relativePath], hidden (IS_PENDING=1) until it is finished.
     * - Older versions: a file at [relativePath] under shared storage.
     *
     * Returns null after asking for the pre-Q storage permission; the invoke
     * is then answered by handlePermissionResult.
     */
    private fun createEntry(
        invoke: Invoke,
        filename: String,
        mimeType: String,
        relativePath: String,
    ): NewEntry? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val resolver = activity.contentResolver
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, filename)
                put(MediaStore.MediaColumns.MIME_TYPE, mimeType)
                put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }
            val uri = resolver.insert(collectionUriFor(mimeType), values)
                ?: throw Exception("Failed to create MediaStore entry for $filename")
            val pfd = try {
                resolver.openFileDescriptor(uri, "w")
            } catch (e: Exception) {
                resolver.delete(uri, null, null)
                throw e
            } ?: run {
                resolver.delete(uri, null, null)
                throw Exception("Failed to open fd for $uri")
            }
            Log.i(TAG, "Created entry: $filename -> $uri")
            return NewEntry(pfd.detachFd(), uri.toString(), uri) // Rust owns the fd now
        }
        if (!hasWritePermission()) {
            pendingPermissionInvoke = invoke
            requestWritePermission()
            return null
        }
        @Suppress("DEPRECATION")
        val dir = File(Environment.getExternalStorageDirectory(), relativePath)
        dir.mkdirs()
        val destFile = File(dir, filename)
        val pfd = android.os.ParcelFileDescriptor.open(
            destFile,
            android.os.ParcelFileDescriptor.MODE_WRITE_ONLY or
                android.os.ParcelFileDescriptor.MODE_CREATE or
                android.os.ParcelFileDescriptor.MODE_TRUNCATE,
        )
        Log.i(TAG, "Created file: ${destFile.absolutePath}")
        return NewEntry(pfd.detachFd(), destFile.absolutePath, null)
    }

    /** Pick the MediaStore collection that matches a MIME type. */
    private fun collectionUriFor(mimeType: String): Uri {
        return when {
            mimeType.startsWith("video/") ->
                MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
            mimeType.startsWith("audio/") ->
                MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
            else ->
                MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        }
    }

    private fun hasWritePermission(): Boolean {
        return ContextCompat.checkSelfPermission(
            activity,
            Manifest.permission.WRITE_EXTERNAL_STORAGE
        ) == PackageManager.PERMISSION_GRANTED
    }

    private fun requestWritePermission() {
        ActivityCompat.requestPermissions(
            activity,
            arrayOf(Manifest.permission.WRITE_EXTERNAL_STORAGE),
            WRITE_STORAGE_REQUEST_CODE
        )
    }
}
