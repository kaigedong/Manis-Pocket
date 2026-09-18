package com.kaigedong.manispocket

import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import java.util.UUID

class ClipboardService(
    private val context: Context,
) {
    private val clipboardManager =
        context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    private var lastClipSig: String = ""
    private val handler = Handler(Looper.getMainLooper())
    private var pollingRunnable: Runnable? = null

    fun startPolling(
        intervalMs: Long = 500L,
        onNewClip: (ClipboardItem) -> Unit,
    ) {
        pollingRunnable =
            object : Runnable {
                override fun run() {
                    val clip = clipboardManager.primaryClip
                    if (clip != null && clip.itemCount > 0) {
                        // Compare actual content, not ClipData identity. ClipData does
                        // not override hashCode(), so the old `clip.hashCode()` check
                        // used the identity hash, which differs on every getPrimaryClip()
                        // call — making the poll fire every interval and re-broadcast the
                        // same clip in a ~0.5s loop.
                        val sig = clipSignature(clip)
                        if (sig.isNotEmpty() && sig != lastClipSig) {
                            lastClipSig = sig
                            val item = clipToItem(clip)
                            if (item != null) {
                                onNewClip(item)
                            }
                        }
                    }
                    handler.postDelayed(this, intervalMs)
                }
            }
        handler.post(pollingRunnable!!)
    }

    private fun clipSignature(clip: android.content.ClipData): String {
        val sb = StringBuilder()
        sb.append(clip.itemCount).append('|')
        for (i in 0 until clip.itemCount) {
            val item = clip.getItemAt(i)
            val repr = item.text?.toString() ?: item.uri?.toString() ?: ""
            sb.append(repr).append('')
        }
        return sb.toString()
    }

    fun stopPolling() {
        pollingRunnable?.let { handler.removeCallbacks(it) }
        pollingRunnable = null
    }

    private fun clipToItem(clip: android.content.ClipData): ClipboardItem? {
        val contents = mutableListOf<ClipboardContent>()
        for (i in 0 until clip.itemCount) {
            val item = clip.getItemAt(i)
            item.text?.let { text ->
                contents.add(
                    ClipboardContent(
                        contentType = "text/plain",
                        value = text.toString().toByteArray(Charsets.UTF_8),
                    ),
                )
            }
            item.uri?.let { uri ->
                contents.add(
                    ClipboardContent(
                        contentType = "text/uri-list",
                        value = uri.toString().toByteArray(Charsets.UTF_8),
                    ),
                )
            }
        }
        if (contents.isEmpty()) return null

        val nowMs = System.currentTimeMillis()
        return ClipboardItem(
            id = UUID.randomUUID().toString(),
            application = null,
            firstCopiedAt = nowMs,
            lastCopiedAt = nowMs,
            numberOfCopies = 1,
            pin = null,
            title =
                contents
                    .firstNotNullOfOrNull { c ->
                        c.value?.toString(Charsets.UTF_8)
                    }?.take(200) ?: "",
            contents = contents,
            syncTimestamp = nowMs,
            syncSource = null,
            syncDeleted = false,
        )
    }

    fun copyToClipboard(item: ClipboardItem) {
        val text =
            item.contents
                .firstOrNull {
                    it.contentType == "text/plain"
                }?.value
                ?.let { String(it, Charsets.UTF_8) } ?: return

        val clip = android.content.ClipData.newPlainText("Manis Pocket", text)
        clipboardManager.setPrimaryClip(clip)
    }
}
