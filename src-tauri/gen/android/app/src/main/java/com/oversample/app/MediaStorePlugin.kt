package com.oversample.app

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.ContentValues
import android.content.Intent
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
data class SaveToSharedArgs(val internalPath: String = "", val filename: String = "", val deleteInternal: Boolean = false)

@InvokeArg
data class SaveWavBytesArgs(val filename: String = "", val data: ByteArray = ByteArray(0))


@InvokeArg
data class CreateRecordingEntryArgs(val filename: String = "")

@InvokeArg
data class ExportFileArgs(val internalPath: String = "", val suggestedName: String = "")

@InvokeArg
data class SaveExportBytesArgs(
    val filename: String = "",
    val data: ByteArray = ByteArray(0),
    val mimeType: String = "application/octet-stream",
    val relativePath: String = "",
)

@TauriPlugin
class MediaStorePlugin(private val activity: Activity) : Plugin(activity) {

    private var pendingPermissionInvoke: Invoke? = null
    private var pendingExportInvoke: Invoke? = null
    private var pendingExportBytes: ByteArray? = null
    private var pendingExportMimeType: String? = null

    // For fd-passing recording: Kotlin holds the MediaStore entry open
    private var pendingRecordingUri: Uri? = null

    companion object {
        const val EXPORT_REQUEST_CODE = 9002
    }

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)
        Log.i(TAG, "MediaStorePlugin loaded")
    }

    // ── Save recording to shared storage ────────────────────────────────

    /**
     * Copy a WAV file from internal app storage to shared storage
     * (Recordings/Oversample on API 29+, Music/Oversample on API 24-28).
     * Returns the public path or content URI string.
     */
    @Command
    fun saveToSharedStorage(invoke: Invoke) {
        val args = invoke.parseArgs(SaveToSharedArgs::class.java)
        val internalFile = File(args.internalPath)
        if (!internalFile.exists()) {
            invoke.reject("File not found: ${args.internalPath}")
            return
        }
        val filename = args.filename.ifEmpty { internalFile.name }

        try {
            val resultPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                saveViaMediaStore(internalFile, filename)
            } else {
                if (!hasWritePermission()) {
                    // Request permission and retry
                    pendingPermissionInvoke = invoke
                    requestWritePermission()
                    return
                }
                saveViaDirectFile(internalFile, filename)
            }
            // Delete internal file after successful move
            if (args.deleteInternal) {
                internalFile.delete()
                Log.i(TAG, "Deleted internal copy: ${args.internalPath}")
            }
            val result = JSObject()
            result.put("path", resultPath)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "saveToSharedStorage failed", e)
            invoke.reject("Failed to save: ${e.message}")
        }
    }

    /**
     * Save WAV bytes directly to shared storage (Recordings/Oversample).
     * Used when the frontend already has the WAV data and doesn't need
     * to go through Rust internal storage first.
     */
    @Command
    fun saveWavBytes(invoke: Invoke) {
        val args = invoke.parseArgs(SaveWavBytesArgs::class.java)
        if (args.data.isEmpty()) {
            invoke.reject("No data provided")
            return
        }
        val filename = args.filename.ifEmpty { "recording.wav" }

        try {
            val resultPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                saveViaMediaStoreBytes(args.data, filename)
            } else {
                if (!hasWritePermission()) {
                    pendingPermissionInvoke = invoke
                    requestWritePermission()
                    return
                }
                saveViaDirectFileBytes(args.data, filename)
            }
            val result = JSObject()
            result.put("path", resultPath)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "saveWavBytes failed", e)
            invoke.reject("Failed to save: ${e.message}")
        }
    }

    /**
     * Save exported file bytes (WAV / MP4 / etc) to shared storage at a
     * given relative path, choosing the MediaStore collection from the MIME
     * type (audio/* → Audio collection, video/* → Video, else Downloads).
     *
     * Unlike a <a download> blob, which the Tauri WebView silently drops,
     * this writes a real file visible in the gallery / Files app. Used by
     * the WAV/MP4 export buttons so exports land somewhere findable.
     */
    @Command
    fun saveExportBytes(invoke: Invoke) {
        val args = invoke.parseArgs(SaveExportBytesArgs::class.java)
        if (args.data.isEmpty()) {
            invoke.reject("No data provided")
            return
        }
        val filename = args.filename.ifEmpty { "export" }

        try {
            val resultPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                saveExportViaMediaStore(args.data, filename, args.mimeType, args.relativePath)
            } else {
                if (!hasWritePermission()) {
                    pendingPermissionInvoke = invoke
                    requestWritePermission()
                    return
                }
                saveExportViaDirectFile(args.data, filename, args.mimeType, args.relativePath)
            }
            val result = JSObject()
            result.put("path", resultPath)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "saveExportBytes failed", e)
            invoke.reject("Failed to save: ${e.message}")
        }
    }

    // ── ContentResolver fd-passing for direct recording ─────────────────

    /**
     * Create a MediaStore entry for a new recording and return a raw POSIX fd
     * that Rust can write WAV data to directly. The entry starts with
     * IS_PENDING=1; call finalizeRecordingEntry when writing is complete.
     *
     * On API < 29, creates the file directly and returns its fd.
     */
    @Command
    fun createRecordingEntry(invoke: Invoke) {
        val args = invoke.parseArgs(CreateRecordingEntryArgs::class.java)
        val filename = args.filename.ifEmpty { "recording.wav" }

        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val resolver = activity.contentResolver
                val values = ContentValues().apply {
                    put(MediaStore.Audio.Media.DISPLAY_NAME, filename)
                    put(MediaStore.Audio.Media.MIME_TYPE, "audio/wav")
                    put(MediaStore.Audio.Media.RELATIVE_PATH, "Recordings/$SUBFOLDER")
                    put(MediaStore.Audio.Media.IS_PENDING, 1)
                }
                val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
                val uri = resolver.insert(collection, values)
                    ?: throw Exception("Failed to create MediaStore entry for $filename")

                pendingRecordingUri = uri

                val pfd = resolver.openFileDescriptor(uri, "w")
                    ?: throw Exception("Failed to open fd for $uri")
                val fd = pfd.detachFd() // caller (Rust) owns the fd now

                Log.i(TAG, "Created recording entry: $filename -> $uri (fd=$fd)")
                val result = JSObject()
                result.put("fd", fd)
                result.put("uri", uri.toString())
                invoke.resolve(result)
            } else {
                // Pre-Q: create file directly
                if (!hasWritePermission()) {
                    pendingPermissionInvoke = invoke
                    requestWritePermission()
                    return
                }
                @Suppress("DEPRECATION")
                val baseDir = File(Environment.getExternalStorageDirectory(), "Recordings")
                val appDir = File(baseDir, SUBFOLDER)
                appDir.mkdirs()
                val destFile = File(appDir, filename)
                // Create empty file and open fd
                destFile.createNewFile()
                val pfd = android.os.ParcelFileDescriptor.open(
                    destFile,
                    android.os.ParcelFileDescriptor.MODE_WRITE_ONLY or android.os.ParcelFileDescriptor.MODE_TRUNCATE
                )
                val fd = pfd.detachFd()

                pendingRecordingUri = null // not needed for pre-Q

                Log.i(TAG, "Created recording file: ${destFile.absolutePath} (fd=$fd)")
                val result = JSObject()
                result.put("fd", fd)
                result.put("uri", destFile.absolutePath)
                invoke.resolve(result)
            }
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
     * Scan MediaStore for our own pending (IS_PENDING=1) entries in
     * Recordings/Oversample and delete them. Called once on app startup to
     * clean up after a crash that left MediaStore rows orphaned — without
     * this the user would see zero-byte "pending" files via other apps that
     * honour IS_PENDING, and they'd accumulate silently over time.
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
            val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
            // Only look inside our subfolder so we don't touch anything else
            // the user might have put in shared Recordings via another app.
            val selection = "${MediaStore.Audio.Media.IS_PENDING} = 1 AND " +
                "${MediaStore.Audio.Media.RELATIVE_PATH} LIKE ?"
            val args = arrayOf("%Recordings/$SUBFOLDER/%")
            val projection = arrayOf(
                MediaStore.Audio.Media._ID,
                MediaStore.Audio.Media.DISPLAY_NAME,
            )

            var deleted = 0
            val names = mutableListOf<String>()
            resolver.query(collection, projection, selection, args, null)?.use { cursor ->
                val idColumn = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media._ID)
                val nameColumn = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media.DISPLAY_NAME)
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

    // ── Export file via SAF picker ───────────────────────────────────────

    /**
     * Open a system "Save As" dialog for the user to choose where to export a file.
     * Works for any file type (WAV, BATM, etc).
     */
    @Command
    fun exportFile(invoke: Invoke) {
        val args = invoke.parseArgs(ExportFileArgs::class.java)
        val file = File(args.internalPath)
        if (!file.exists()) {
            invoke.reject("File not found: ${args.internalPath}")
            return
        }
        val suggestedName = args.suggestedName.ifEmpty { file.name }
        val mimeType = when {
            suggestedName.endsWith(".wav", ignoreCase = true) -> "audio/wav"
            suggestedName.endsWith(".flac", ignoreCase = true) -> "audio/flac"
            suggestedName.endsWith(".mp3", ignoreCase = true) -> "audio/mpeg"
            suggestedName.endsWith(".ogg", ignoreCase = true) -> "audio/ogg"
            suggestedName.endsWith(".batm", ignoreCase = true) -> "application/octet-stream"
            suggestedName.endsWith(".yaml", ignoreCase = true) -> "text/yaml"
            suggestedName.endsWith(".yml", ignoreCase = true) -> "text/yaml"
            else -> "application/octet-stream"
        }

        pendingExportInvoke = invoke
        pendingExportBytes = file.readBytes()
        pendingExportMimeType = mimeType

        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = mimeType
            putExtra(Intent.EXTRA_TITLE, suggestedName)
        }
        activity.startActivityForResult(intent, EXPORT_REQUEST_CODE)
    }

    // ── Activity result handling (SAF) ──────────────────────────────────

    fun handleActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode != EXPORT_REQUEST_CODE) return

        val invoke = pendingExportInvoke ?: return
        pendingExportInvoke = null
        val bytes = pendingExportBytes
        pendingExportBytes = null
        pendingExportMimeType = null

        if (resultCode != Activity.RESULT_OK || data?.data == null) {
            val result = JSObject()
            result.put("cancelled", true)
            invoke.resolve(result)
            return
        }

        val uri = data.data!!
        try {
            activity.contentResolver.openOutputStream(uri)?.use { out ->
                out.write(bytes)
            }
            val result = JSObject()
            result.put("cancelled", false)
            result.put("uri", uri.toString())
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "SAF export failed", e)
            invoke.reject("Export failed: ${e.message}")
        }
    }

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

    /** API 29+: Save via MediaStore to Recordings/Oversample */
    private fun saveViaMediaStore(sourceFile: File, displayName: String): String {
        val resolver = activity.contentResolver

        val values = ContentValues().apply {
            put(MediaStore.Audio.Media.DISPLAY_NAME, displayName)
            put(MediaStore.Audio.Media.MIME_TYPE, "audio/wav")
            put(MediaStore.Audio.Media.RELATIVE_PATH, "Recordings/$SUBFOLDER")
            put(MediaStore.Audio.Media.IS_PENDING, 1)
        }

        val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(collection, values)
            ?: throw Exception("Failed to create MediaStore entry for $displayName")

        resolver.openOutputStream(uri)?.use { outputStream ->
            sourceFile.inputStream().use { inputStream ->
                inputStream.copyTo(outputStream, bufferSize = 8192)
            }
        } ?: throw Exception("Failed to open output stream for $displayName")

        // Mark as complete
        values.clear()
        values.put(MediaStore.Audio.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)

        Log.i(TAG, "Saved to MediaStore: $displayName -> $uri")
        return uri.toString()
    }

    /** API 24-28: Save via direct file I/O to shared storage */
    @Suppress("DEPRECATION")
    private fun saveViaDirectFile(sourceFile: File, displayName: String): String {
        // Use Recordings/ directory (create it if needed, works fine on pre-Q)
        val baseDir = File(Environment.getExternalStorageDirectory(), "Recordings")
        val appDir = File(baseDir, SUBFOLDER)
        appDir.mkdirs()

        val destFile = File(appDir, displayName)
        sourceFile.inputStream().use { input ->
            destFile.outputStream().use { output ->
                input.copyTo(output, bufferSize = 8192)
            }
        }

        // Notify MediaStore so it appears in media apps / file managers
        MediaScannerConnection.scanFile(
            activity,
            arrayOf(destFile.absolutePath),
            arrayOf("audio/wav"),
            null
        )

        Log.i(TAG, "Saved to file: ${destFile.absolutePath}")
        return destFile.absolutePath
    }

    /** API 29+: Save raw bytes via MediaStore to Recordings/Oversample */
    private fun saveViaMediaStoreBytes(data: ByteArray, displayName: String): String {
        val resolver = activity.contentResolver

        val values = ContentValues().apply {
            put(MediaStore.Audio.Media.DISPLAY_NAME, displayName)
            put(MediaStore.Audio.Media.MIME_TYPE, "audio/wav")
            put(MediaStore.Audio.Media.RELATIVE_PATH, "Recordings/$SUBFOLDER")
            put(MediaStore.Audio.Media.IS_PENDING, 1)
        }

        val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(collection, values)
            ?: throw Exception("Failed to create MediaStore entry for $displayName")

        resolver.openOutputStream(uri)?.use { outputStream ->
            outputStream.write(data)
        } ?: throw Exception("Failed to open output stream for $displayName")

        values.clear()
        values.put(MediaStore.Audio.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)

        Log.i(TAG, "Saved bytes to MediaStore: $displayName -> $uri")
        return uri.toString()
    }

    /** API 24-28: Save raw bytes via direct file I/O to shared storage */
    @Suppress("DEPRECATION")
    private fun saveViaDirectFileBytes(data: ByteArray, displayName: String): String {
        val baseDir = File(Environment.getExternalStorageDirectory(), "Recordings")
        val appDir = File(baseDir, SUBFOLDER)
        appDir.mkdirs()

        val destFile = File(appDir, displayName)
        destFile.writeBytes(data)

        MediaScannerConnection.scanFile(
            activity,
            arrayOf(destFile.absolutePath),
            arrayOf("audio/wav"),
            null
        )

        Log.i(TAG, "Saved bytes to file: ${destFile.absolutePath}")
        return destFile.absolutePath
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

    /** API 29+: Save raw export bytes via MediaStore into the collection/folder
     *  implied by the MIME type and relative path. */
    private fun saveExportViaMediaStore(
        data: ByteArray,
        displayName: String,
        mimeType: String,
        relativePath: String,
    ): String {
        val resolver = activity.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mimeType)
            if (relativePath.isNotEmpty()) {
                put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
            }
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val collection = collectionUriFor(mimeType)
        val uri = resolver.insert(collection, values)
            ?: throw Exception("Failed to create MediaStore entry for $displayName")
        resolver.openOutputStream(uri)?.use { it.write(data) }
            ?: throw Exception("Failed to open output stream for $displayName")
        values.clear()
        values.put(MediaStore.MediaColumns.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
        Log.i(TAG, "Saved export bytes to MediaStore: $displayName -> $uri")
        return uri.toString()
    }

    /** API 24-28: Save raw export bytes via direct file I/O into a shared folder. */
    @Suppress("DEPRECATION")
    private fun saveExportViaDirectFile(
        data: ByteArray,
        displayName: String,
        mimeType: String,
        relativePath: String,
    ): String {
        val rel = relativePath.ifEmpty { "Download/$SUBFOLDER/exports" }
        val baseDir = File(Environment.getExternalStorageDirectory(), rel)
        baseDir.mkdirs()
        val destFile = File(baseDir, displayName)
        destFile.writeBytes(data)
        MediaScannerConnection.scanFile(
            activity,
            arrayOf(destFile.absolutePath),
            arrayOf(mimeType),
            null
        )
        Log.i(TAG, "Saved export bytes to file: ${destFile.absolutePath}")
        return destFile.absolutePath
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
