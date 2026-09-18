package com.kaigedong.manispocket

import android.os.Environment
import java.io.File

/** Assembles file chunks received via P2P and saves to Downloads. */
object FileDownloadManager {
    private data class Download(
        val requestId: String,
        val fileName: String,
        val totalSize: Long,
        val chunks: MutableList<ByteArray> = mutableListOf(),
    )

    private val downloads = mutableMapOf<String, Download>()

    @Synchronized
    fun receiveChunk(
        requestId: String,
        fileName: String,
        fileSize: Long,
        chunkIndex: Int,
        totalChunks: Int,
        data: ByteArray,
    ) {
        downloads.getOrPut(requestId) { Download(requestId, fileName, fileSize) }.chunks.add(data)
    }

    @Synchronized
    fun complete(
        requestId: String,
        filePath: String,
        success: Boolean,
    ) {
        val dl = downloads.remove(requestId) ?: return
        if (!success) {
            LogManager.e("FileTransfer", "Download failed: ${dl.fileName}")
            return
        }

        try {
            val dir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val file = File(dir, dl.fileName)
            file.outputStream().use { out ->
                for (chunk in dl.chunks) {
                    out.write(chunk)
                }
            }
            LogManager.i("FileTransfer", "Downloaded ${dl.fileName} (${dl.chunks.sumOf { it.size }} bytes) to ${file.absolutePath}")
        } catch (e: Exception) {
            LogManager.e("FileTransfer", "Failed to save ${dl.fileName}", e)
        }
    }
}
