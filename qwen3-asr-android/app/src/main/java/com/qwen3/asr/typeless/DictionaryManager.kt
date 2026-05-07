package com.qwen3.asr.typeless

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/**
 * A single dictionary entry representing a custom word and its preferred spelling.
 */
data class DictionaryEntry(
    /** Unique identifier (UUID v4). */
    val id: String,
    /** The word or phrase as it might appear in ASR output. */
    val word: String,
    /** The preferred correct spelling to use. */
    val correctSpelling: String,
    /** Optional category (e.g. "medical", "legal", "tech"). */
    val category: String?
)

/**
 * Manages personal dictionary entries with persistence via SharedPreferences + JSON.
 *
 * Dictionary words can be injected into LLM post-processing prompts
 * to improve recognition of domain-specific terms.
 */
class DictionaryManager(context: Context) {

    private val prefs: SharedPreferences by lazy {
        context.getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
    }

    /** In-memory cache, loaded lazily from SharedPreferences on first access. */
    private val entries: MutableList<DictionaryEntry> by lazy { loadEntries() }

    companion object {
        private const val PREF_KEY = "dictionary_entries"
        private const val KEY_ID = "id"
        private const val KEY_WORD = "word"
        private const val KEY_CORRECT_SPELLING = "correct_spelling"
        private const val KEY_CATEGORY = "category"
    }

    /**
     * Load entries from SharedPreferences. Returns an empty list if nothing stored.
     */
    private fun loadEntries(): MutableList<DictionaryEntry> {
        val json = prefs.getString(PREF_KEY, null) ?: return mutableListOf()
        return parseEntries(json)
    }

    /**
     * Parse a JSON array string into a list of [DictionaryEntry].
     */
    private fun parseEntries(json: String): MutableList<DictionaryEntry> {
        val result = mutableListOf<DictionaryEntry>()
        val array = JSONArray(json)
        for (i in 0 until array.length()) {
            val obj = array.getJSONObject(i)
            result.add(
                DictionaryEntry(
                    id = obj.getString(KEY_ID),
                    word = obj.getString(KEY_WORD),
                    correctSpelling = obj.getString(KEY_CORRECT_SPELLING),
                    category = obj.optString(KEY_CATEGORY, "")
                )
            )
        }
        return result
    }

    /**
     * Persist current entries to SharedPreferences as a JSON string.
     */
    private fun save() {
        val array = JSONArray()
        for (entry in entries) {
            val obj = JSONObject().apply {
                put(KEY_ID, entry.id)
                put(KEY_WORD, entry.word)
                put(KEY_CORRECT_SPELLING, entry.correctSpelling)
                if (entry.category != null) {
                    put(KEY_CATEGORY, entry.category)
                }
            }
            array.put(obj)
        }
        prefs.edit().putString(PREF_KEY, array.toString()).apply()
    }

    /**
     * Return all entries.
     */
    fun listEntries(): List<DictionaryEntry> = entries.toList()

    /**
     * Add a new dictionary entry. Returns the created entry.
     * @throws IllegalArgumentException if a duplicate word already exists (case-insensitive).
     */
    fun addEntry(word: String, correctSpelling: String, category: String?): DictionaryEntry {
        val wordLower = word.lowercase()
        if (entries.any { it.word.lowercase() == wordLower }) {
            throw IllegalArgumentException("Word '$word' already exists in dictionary")
        }
        val entry = DictionaryEntry(
            id = UUID.randomUUID().toString(),
            word = word,
            correctSpelling = correctSpelling,
            category = category
        )
        entries.add(entry)
        save()
        return entry
    }

    /**
     * Remove an entry by its ID. No-op if the ID is not found.
     */
    fun removeEntry(id: String) {
        val removed = entries.removeAll { it.id == id }
        if (removed) {
            save()
        }
    }

    /**
     * Format dictionary entries as "word → correctSpelling" lines for LLM prompt injection.
     * Returns an empty string if there are no entries.
     */
    fun formatForPrompt(): String {
        if (entries.isEmpty()) return ""
        return entries.joinToString("\n") { "${it.word} → ${it.correctSpelling}" }
    }

    /**
     * Export all entries as a JSON array string.
     */
    fun exportToJson(): String {
        val array = JSONArray()
        for (entry in entries) {
            val obj = JSONObject().apply {
                put(KEY_ID, entry.id)
                put(KEY_WORD, entry.word)
                put(KEY_CORRECT_SPELLING, entry.correctSpelling)
                if (entry.category != null) {
                    put(KEY_CATEGORY, entry.category)
                }
            }
            array.put(obj)
        }
        return array.toString(2)
    }

    /**
     * Import entries from a JSON string, merging with existing entries.
     * Skips duplicates (by word, case-insensitive).
     */
    fun importFromJson(json: String) {
        val imported = parseEntries(json)
        var added = false
        for (entry in imported) {
            val wordLower = entry.word.lowercase()
            if (entries.none { it.word.lowercase() == wordLower }) {
                entries.add(entry)
                added = true
            }
        }
        if (added) {
            save()
        }
    }

    /**
     * Return the number of entries.
     */
    fun count(): Int = entries.size
}
