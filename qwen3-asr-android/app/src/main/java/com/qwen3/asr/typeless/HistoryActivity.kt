package com.qwen3.asr.typeless

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.LayoutInflater
import android.view.MenuItem
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.google.android.material.appbar.MaterialToolbar
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.OutputStreamWriter
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * History viewer activity.
 *
 * Features:
 *  - RecyclerView with history entries
 *  - Each item shows: timestamp, text preview, mode badge, duration
 *  - Click to view full text with copy/share buttons
 *  - Swipe to delete
 *  - Search bar at top
 *  - Export all as JSON / CSV / TXT via Storage Access Framework
 *  - Share individual entries via Android share intent
 */
class HistoryActivity : AppCompatActivity() {

    companion object {
        private const val REQUEST_EXPORT_JSON = 1001
        private const val REQUEST_EXPORT_CSV = 1002
        private const val REQUEST_EXPORT_TXT = 1003
    }

    private lateinit var toolbar: MaterialToolbar
    private lateinit var etSearch: EditText
    private lateinit var recyclerView: RecyclerView
    private lateinit var adapter: HistoryAdapter

    private var allEntries: List<HistoryEntry> = emptyList()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_history)

        toolbar = findViewById(R.id.toolbar)
        etSearch = findViewById(R.id.et_search)
        recyclerView = findViewById(R.id.recycler_history)

        toolbar.setNavigationOnClickListener { finish() }
        toolbar.setOnMenuItemClickListener { item -> onToolbarItemSelected(item) }

        adapter = HistoryAdapter(
            onClick = { entry -> showDetailDialog(entry) },
            onDelete = { entry -> deleteEntry(entry) }
        )

        recyclerView.layoutManager = LinearLayoutManager(this)
        recyclerView.adapter = adapter

        loadHistory()

        etSearch.addTextChangedListener(object : android.text.TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                filterHistory(s?.toString() ?: "")
            }
            override fun afterTextChanged(s: android.text.Editable?) {}
        })
    }

    override fun onResume() {
        super.onResume()
        loadHistory()
    }

    // ---------- Toolbar menu ----------

    private fun onToolbarItemSelected(item: MenuItem): Boolean {
        return when (item.itemId) {
            R.id.action_export_all,
            R.id.action_export_json -> {
                launchExportIntent("json")
                true
            }
            R.id.action_export_csv -> {
                launchExportIntent("csv")
                true
            }
            R.id.action_export_txt -> {
                launchExportIntent("txt")
                true
            }
            else -> false
        }
    }

    /**
     * Launch SAF ACTION_CREATE_DOCUMENT intent for the given format.
     */
    private fun launchExportIntent(format: String) {
        val dateFormat = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.getDefault())
        val timestamp = dateFormat.format(Date())
        val fileName = "history_export_$timestamp.$format"

        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = when (format) {
                "json" -> "application/json"
                "csv" -> "text/csv"
                else -> "text/plain"
            }
            putExtra(Intent.EXTRA_TITLE, fileName)
        }
        val requestCode = when (format) {
            "json" -> REQUEST_EXPORT_JSON
            "csv" -> REQUEST_EXPORT_CSV
            else -> REQUEST_EXPORT_TXT
        }
        @Suppress("DEPRECATION")
        startActivityForResult(intent, requestCode)
    }

    @Deprecated("Needed for SAF ACTION_CREATE_DOCUMENT result")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)

        if (resultCode != RESULT_OK || data == null) return

        val uri: Uri = data.data ?: return
        val format = when (requestCode) {
            REQUEST_EXPORT_JSON -> "json"
            REQUEST_EXPORT_CSV -> "csv"
            REQUEST_EXPORT_TXT -> "txt"
            else -> return
        }

        writeExport(uri, format)
    }

    /**
     * Write the export data to the SAF-provided [uri] in the specified [format].
     */
    private fun writeExport(uri: Uri, format: String) {
        val entries = allEntries
        if (entries.isEmpty()) {
            Toast.makeText(this, getString(R.string.history_no_entries_to_export), Toast.LENGTH_SHORT).show()
            return
        }

        lifecycleScope.launch {
            val content = withContext(Dispatchers.Default) {
                when (format) {
                    "json" -> exportAsJson(entries)
                    "csv" -> exportAsCsv(entries)
                    else -> exportAsTxt(entries)
                }
            }

            withContext(Dispatchers.IO) {
                try {
                    contentResolver.openOutputStream(uri)?.use { os ->
                        OutputStreamWriter(os, "UTF-8").use { writer ->
                            writer.write(content)
                        }
                    }
                } catch (e: Exception) {
                    withContext(Dispatchers.Main) {
                        Toast.makeText(
                            this@HistoryActivity,
                            getString(R.string.history_export_failed, e.message),
                            Toast.LENGTH_LONG
                        ).show()
                    }
                    return@withContext
                }
            }

            withContext(Dispatchers.Main) {
                Toast.makeText(
                    this@HistoryActivity,
                    getString(R.string.history_exported, entries.size, format),
                    Toast.LENGTH_SHORT
                ).show()
            }
        }
    }

    // ---------- Export formatters ----------

    /**
     * Export history entries as a JSON array.
     */
    private fun exportAsJson(entries: List<HistoryEntry>): String {
        val array = JSONArray()
        for (entry in entries) {
            val obj = JSONObject().apply {
                put("timestamp", entry.timestamp)
                put("text", entry.text)
                put("raw_text", entry.rawText)
                put("mode", entry.mode)
                put("duration", entry.durationSecs)
                put("language", entry.language)
            }
            array.put(obj)
        }
        return array.toString(2)
    }

    /**
     * Export history entries as CSV with headers: timestamp, text, raw_text, mode, duration, language.
     */
    private fun exportAsCsv(entries: List<HistoryEntry>): String {
        val sb = StringBuilder()
        sb.appendLine("timestamp,text,raw_text,mode,duration,language")
        for (entry in entries) {
            sb.appendLine(
                "${entry.timestamp}," +
                        "\"${entry.text.replace("\"", "\"\"")}\"," +
                        "\"${entry.rawText.replace("\"", "\"\"")}\"," +
                        "${entry.mode}," +
                        "${entry.durationSecs}," +
                        "${entry.language}"
            )
        }
        return sb.toString()
    }

    /**
     * Export history entries as plain text, one entry per line with timestamp prefix.
     */
    private fun exportAsTxt(entries: List<HistoryEntry>): String {
        val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
        val sb = StringBuilder()
        for (entry in entries) {
            val dateStr = dateFormat.format(Date(entry.timestamp))
            sb.appendLine("[$dateStr] ${entry.text}")
        }
        return sb.toString()
    }

    // ---------- Data loading ----------

    private fun loadHistory() {
        lifecycleScope.launch {
            allEntries = withContext(Dispatchers.IO) {
                HistoryDatabase.getInstance(this@HistoryActivity)
                    .historyDao()
                    .getAll()
            }
            withContext(Dispatchers.Main) {
                adapter.submitList(allEntries)
            }
        }
    }

    private fun filterHistory(query: String) {
        if (query.isBlank()) {
            adapter.submitList(allEntries)
            return
        }

        val filtered = allEntries.filter {
            it.text.contains(query, ignoreCase = true)
        }
        adapter.submitList(filtered)
    }

    private fun showDetailDialog(entry: HistoryEntry) {
        val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
        val dateStr = dateFormat.format(Date(entry.timestamp))

        AlertDialog.Builder(this)
            .setTitle(dateStr)
            .setMessage(entry.text)
            .setPositiveButton(getString(R.string.history_copy)) { _, _ ->
                val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                clipboard.setPrimaryClip(android.content.ClipData.newPlainText("ASR", entry.text))
                Toast.makeText(this, getString(R.string.copied_to_clipboard), Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton(getString(R.string.history_close), null)
            .show()
    }

    private fun deleteEntry(entry: HistoryEntry) {
        lifecycleScope.launch {
            withContext(Dispatchers.IO) {
                HistoryDatabase.getInstance(this@HistoryActivity)
                    .historyDao()
                    .delete(entry)
            }
            loadHistory()
        }
    }

    // ---------- RecyclerView Adapter ----------

    inner class HistoryAdapter(
        private val onClick: (HistoryEntry) -> Unit,
        private val onDelete: (HistoryEntry) -> Unit
    ) : RecyclerView.Adapter<HistoryAdapter.ViewHolder>() {

        private var entries: List<HistoryEntry> = emptyList()

        fun submitList(list: List<HistoryEntry>) {
            entries = list
            notifyDataSetChanged()
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): ViewHolder {
            val view = LayoutInflater.from(parent.context)
                .inflate(R.layout.item_history, parent, false)
            return ViewHolder(view)
        }

        override fun onBindViewHolder(holder: ViewHolder, position: Int) {
            val entry = entries[position]
            holder.bind(entry)
        }

        override fun getItemCount(): Int = entries.size

        inner class ViewHolder(view: View) : RecyclerView.ViewHolder(view) {
            private val tvTimestamp: TextView = view.findViewById(R.id.tv_timestamp)
            private val tvPreview: TextView = view.findViewById(R.id.tv_preview)
            private val tvMode: TextView = view.findViewById(R.id.tv_mode)
            private val tvDuration: TextView = view.findViewById(R.id.tv_duration)

            fun bind(entry: HistoryEntry) {
                val dateFormat = SimpleDateFormat("HH:mm:ss", Locale.getDefault())
                tvTimestamp.text = dateFormat.format(Date(entry.timestamp))

                // Truncate preview
                val preview = if (entry.text.length > 80) {
                    entry.text.substring(0, 80) + "..."
                } else {
                    entry.text
                }
                tvPreview.text = preview

                tvMode.text = when (entry.mode) {
                    RecordingService.MODE_PTT -> getString(R.string.mode_ptt)
                    RecordingService.MODE_HANDSFREE -> getString(R.string.mode_vad)
                    else -> entry.mode
                }

                tvDuration.text = String.format("%.1fs", entry.durationSecs)

                itemView.setOnClickListener { onClick(entry) }

                itemView.setOnLongClickListener {
                    AlertDialog.Builder(itemView.context)
                        .setTitle(itemView.context.getString(R.string.history_delete_entry))
                        .setMessage(itemView.context.getString(R.string.history_delete_confirmation))
                        .setPositiveButton(itemView.context.getString(R.string.history_delete)) { _, _ -> onDelete(entry) }
                        .setNegativeButton(itemView.context.getString(R.string.dict_cancel), null)
                        .show()
                    true
                }
            }
        }
    }
}
