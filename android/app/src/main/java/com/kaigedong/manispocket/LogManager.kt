package com.kaigedong.manispocket

import android.os.Environment
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.io.File
import java.io.PrintWriter
import java.io.StringWriter
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

object LogManager {
    data class LogEntry(
        val timestamp: Long,
        val level: String,
        val tag: String,
        val message: String,
    ) {
        fun formatted(): String {
            val sdf = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)
            return "${sdf.format(Date(timestamp))} [$level] $tag: $message"
        }
    }

    private val _logs = MutableStateFlow<List<LogEntry>>(emptyList())
    val logs: StateFlow<List<LogEntry>> = _logs

    // In-memory buffer for copy/export
    private val buffer = mutableListOf<LogEntry>()
    private const val MAX_BUFFER = 2000

    fun d(
        tag: String,
        message: String,
    ) = add(LogEntry(System.currentTimeMillis(), "DEBUG", tag, message))

    fun i(
        tag: String,
        message: String,
    ) = add(LogEntry(System.currentTimeMillis(), "INFO", tag, message))

    fun w(
        tag: String,
        message: String,
    ) = add(LogEntry(System.currentTimeMillis(), "WARN", tag, message))

    fun e(
        tag: String,
        message: String,
        throwable: Throwable? = null,
    ) {
        val msg =
            if (throwable != null) {
                "$message\n${stackTraceToString(throwable)}"
            } else {
                message
            }
        add(LogEntry(System.currentTimeMillis(), "ERROR", tag, msg))
    }

    @Synchronized
    private fun add(entry: LogEntry) {
        buffer.add(entry)
        if (buffer.size > MAX_BUFFER) {
            buffer.removeAt(0)
        }
        _logs.value = buffer.toList()
    }

    /** Copy all logs to clipboard as plain text */
    fun toText(): String = buffer.joinToString("\n") { it.formatted() }

    /** Export logs to a file in the Downloads directory */
    fun exportToFile(): File? =
        try {
            val dir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val sdf = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US)
            val file = File(dir, "manispocket_logs_${sdf.format(Date())}.txt")
            file.writeText(toText())
            file
        } catch (e: Exception) {
            null
        }

    fun clear() {
        buffer.clear()
        _logs.value = emptyList()
    }

    private fun stackTraceToString(t: Throwable): String {
        val sw = StringWriter()
        t.printStackTrace(PrintWriter(sw))
        return sw.toString()
    }
}
