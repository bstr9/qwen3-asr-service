package com.qwen3.asr.typeless

import android.content.Context
import android.os.Bundle
import android.view.LayoutInflater
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
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * History viewer activity.
 *
 * Features:
 *  - RecyclerView with history entries
 *  - Each item shows: timestamp, text preview, mode badge, duration
 *  - Click to view full text with copy button
 *  - Swipe to delete
 *  - Search bar at top
 */
class HistoryActivity : AppCompatActivity() {

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
            .setPositiveButton("Copy") { _, _ ->
                val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                clipboard.setPrimaryClip(android.content.ClipData.newPlainText("ASR", entry.text))
                Toast.makeText(this, "Copied to clipboard", Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton("Close", null)
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
                    RecordingService.MODE_PTT -> "PTT"
                    RecordingService.MODE_HANDSFREE -> "VAD"
                    else -> entry.mode
                }

                tvDuration.text = String.format("%.1fs", entry.durationSecs)

                itemView.setOnClickListener { onClick(entry) }

                itemView.setOnLongClickListener {
                    AlertDialog.Builder(itemView.context)
                        .setTitle("Delete entry?")
                        .setMessage("Delete this transcription?")
                        .setPositiveButton("Delete") { _, _ -> onDelete(entry) }
                        .setNegativeButton("Cancel", null)
                        .show()
                    true
                }
            }
        }
    }
}
