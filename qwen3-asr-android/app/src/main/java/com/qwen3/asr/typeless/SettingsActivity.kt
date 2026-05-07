package com.qwen3.asr.typeless

import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.net.Uri
import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.TextView
import android.widget.Toast
import com.google.android.material.switchmaterial.SwitchMaterial
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.appbar.MaterialToolbar
import kotlinx.coroutines.Dispatchers
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Settings screen for ASR service configuration.
 *
 * Settings:
 *  - ASR URL (EditText, default http://192.168.1.100:8765)
 *  - API Key (EditText, password type)
 *  - Default mode (dropdown: PTT / Hands-free)
 *  - VAD threshold (SeekBar 0.0–1.0)
 *  - Silence duration (EditText, seconds)
 *  - Post-processing toggle (with sub-options: remove fillers, remove repetitions, auto-format)
 *  - Dictionary management (add/remove entries via DictionaryManager)
 *  - Dictionary import/export (JSON via Storage Access Framework)
 *  - LLM post-processing (URL, model, API key, custom prompt)
 *
 * All settings saved to SharedPreferences.
 */
class SettingsActivity : AppCompatActivity() {

    companion object {
        private const val REQUEST_DICT_EXPORT = 2001
        private const val REQUEST_DICT_IMPORT = 2002
    }

    private val prefs: SharedPreferences by lazy {
        getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
    }

    private lateinit var toolbar: MaterialToolbar
    private lateinit var etAsrUrl: EditText
    private lateinit var etApiKey: EditText
    private lateinit var spinnerMode: Spinner
    private lateinit var seekVadThreshold: SeekBar
    private lateinit var tvVadThresholdValue: EditText
    private lateinit var etSilenceDuration: EditText
    private lateinit var switchPostProcessing: SwitchMaterial
    private lateinit var switchRemoveFillers: SwitchMaterial
    private lateinit var switchRemoveRepetitions: SwitchMaterial
    private lateinit var switchAutoFormat: SwitchMaterial
    private lateinit var etMaxRecordingDuration: EditText
    private lateinit var switchPlaySounds: SwitchMaterial
    private lateinit var btnAddWord: Button
    private lateinit var dictionaryList: LinearLayout
    private lateinit var btnExportDictionary: Button
    private lateinit var btnImportDictionary: Button
    private lateinit var switchLlmEnabled: SwitchMaterial
    private lateinit var etLlmUrl: EditText
    private lateinit var etLlmModel: EditText
    private lateinit var etLlmApiKey: EditText
    private lateinit var etCustomPrompt: EditText

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_settings)

        initViews()
        loadSettings()
        setupListeners()
    }

    private fun initViews() {
        toolbar = findViewById(R.id.toolbar)
        etAsrUrl = findViewById(R.id.et_asr_url)
        etApiKey = findViewById(R.id.et_api_key)
        spinnerMode = findViewById(R.id.spinner_mode)
        seekVadThreshold = findViewById(R.id.seek_vad_threshold)
        tvVadThresholdValue = findViewById(R.id.tv_vad_threshold_value)
        etSilenceDuration = findViewById(R.id.et_silence_duration)
        switchPostProcessing = findViewById(R.id.switch_post_processing)

        // Recording settings
        etMaxRecordingDuration = findViewById(R.id.et_max_recording_duration)
        switchPlaySounds = findViewById(R.id.switch_play_sounds)

        // Post-processing sub-options
        switchRemoveFillers = findViewById(R.id.switch_remove_fillers)
        switchRemoveRepetitions = findViewById(R.id.switch_remove_repetitions)
        switchAutoFormat = findViewById(R.id.switch_auto_format)

        // Dictionary management
        btnAddWord = findViewById(R.id.btn_add_word)
        dictionaryList = findViewById(R.id.dictionary_list)
        btnExportDictionary = findViewById(R.id.btn_export_dictionary)
        btnImportDictionary = findViewById(R.id.btn_import_dictionary)

        // LLM post-processing
        switchLlmEnabled = findViewById(R.id.switch_llm_enabled)
        etLlmUrl = findViewById(R.id.et_llm_url)
        etLlmModel = findViewById(R.id.et_llm_model)
        etLlmApiKey = findViewById(R.id.et_llm_api_key)
        etCustomPrompt = findViewById(R.id.et_custom_prompt)

        toolbar.setNavigationOnClickListener { finish() }

        // Mode dropdown
        val modes = arrayOf(getString(R.string.mode_ptt_full), getString(R.string.mode_handsfree_full))
        val adapter = ArrayAdapter(this, android.R.layout.simple_spinner_dropdown_item, modes)
        spinnerMode.adapter = adapter

        // VAD threshold SeekBar: 0-100 mapped to 0.0-1.0
        seekVadThreshold.max = 100

        // Dictionary: add word button
        btnAddWord.setOnClickListener { showAddWordDialog() }

        // Dictionary: export button
        btnExportDictionary.setOnClickListener { launchDictionaryExport() }

        // Dictionary: import button
        btnImportDictionary.setOnClickListener { launchDictionaryImport() }

        // Post-processing toggle controls sub-option visibility
        switchPostProcessing.setOnCheckedChangeListener { _, isChecked ->
            switchRemoveFillers.isEnabled = isChecked
            switchRemoveRepetitions.isEnabled = isChecked
            switchAutoFormat.isEnabled = isChecked
        }
    }

    private fun loadSettings() {
        etAsrUrl.setText(prefs.getString("asr_url", "http://192.168.1.100:8765"))
        etApiKey.setText(prefs.getString("api_key", ""))

        val mode = prefs.getString("default_mode", RecordingService.MODE_PTT)
        spinnerMode.setSelection(if (mode == RecordingService.MODE_HANDSFREE) 1 else 0)

        val vadThreshold = prefs.getFloat("vad_threshold", VadDetector.DEFAULT_THRESHOLD)
        seekVadThreshold.progress = (vadThreshold * 100).toInt()
        tvVadThresholdValue.setText(String.format("%.2f", vadThreshold))

        etSilenceDuration.setText(
            prefs.getFloat("silence_duration", VadDetector.DEFAULT_SILENCE_DURATION_SEC).toString()
        )

        switchPostProcessing.isChecked = prefs.getBoolean("post_processing", true)

        // Post-processing sub-options
        switchRemoveFillers.isChecked = prefs.getBoolean("remove_fillers", true)
        switchRemoveRepetitions.isChecked = prefs.getBoolean("remove_repetitions", true)
        switchAutoFormat.isChecked = prefs.getBoolean("auto_format", true)
        val postEnabled = switchPostProcessing.isChecked
        switchRemoveFillers.isEnabled = postEnabled
        switchRemoveRepetitions.isEnabled = postEnabled
        switchAutoFormat.isEnabled = postEnabled

        // Recording settings
        etMaxRecordingDuration.setText(
            prefs.getInt("max_recording_duration", 60).toString()
        )
        switchPlaySounds.isChecked = prefs.getBoolean("play_sounds", true)

        // Dictionary
        refreshDictionaryList()

        // LLM post-processing
        switchLlmEnabled.isChecked = prefs.getBoolean("llm_enabled", false)
        etLlmUrl.setText(prefs.getString("llm_url", ""))
        etLlmModel.setText(prefs.getString("llm_model", ""))
        etLlmApiKey.setText(prefs.getString("llm_api_key", ""))
        etCustomPrompt.setText(prefs.getString("custom_prompt", ""))
    }

    private fun setupListeners() {
        seekVadThreshold.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                val value = progress / 100f
                tvVadThresholdValue.setText(String.format("%.2f", value))
            }
            override fun onStartTrackingTouch(seekBar: SeekBar?) {}
            override fun onStopTrackingTouch(seekBar: SeekBar?) {
                saveSettings()
            }
        })
    }

    private fun saveSettings() {
        prefs.edit().apply {
            putString("asr_url", etAsrUrl.text.toString().trim())
            putString("api_key", etApiKey.text.toString().trim())

            val mode = if (spinnerMode.selectedItemPosition == 1)
                RecordingService.MODE_HANDSFREE else RecordingService.MODE_PTT
            putString("default_mode", mode)

            putFloat("vad_threshold", seekVadThreshold.progress / 100f)

            val silenceDur = etSilenceDuration.text.toString().toFloatOrNull()
                ?: VadDetector.DEFAULT_SILENCE_DURATION_SEC
            putFloat("silence_duration", silenceDur)

            putBoolean("post_processing", switchPostProcessing.isChecked)

            // Post-processing sub-options
            putBoolean("remove_fillers", switchRemoveFillers.isChecked)
            putBoolean("remove_repetitions", switchRemoveRepetitions.isChecked)
            putBoolean("auto_format", switchAutoFormat.isChecked)

            // Recording settings
            val maxDuration = etMaxRecordingDuration.text.toString().toIntOrNull() ?: 60
            putInt("max_recording_duration", maxDuration)
            putBoolean("play_sounds", switchPlaySounds.isChecked)

            // LLM post-processing
            putBoolean("llm_enabled", switchLlmEnabled.isChecked)
            putString("llm_url", etLlmUrl.text.toString().trim())
            putString("llm_model", etLlmModel.text.toString().trim())
            putString("llm_api_key", etLlmApiKey.text.toString().trim())
            putString("custom_prompt", etCustomPrompt.text.toString().trim())

            apply()
        }
    }

    // ---------- Dictionary management ----------

    private fun refreshDictionaryList() {
        dictionaryList.removeAllViews()
        val dictManager = DictionaryManager(this)
        val entries = dictManager.listEntries()
        for (entry in entries) {
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                )
                setPadding(0, 4, 0, 4)
            }

            val label = TextView(this).apply {
                text = "${entry.word} → ${entry.correctSpelling}"
                layoutParams = LinearLayout.LayoutParams(
                    0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f
                )
                setTextAppearance(android.R.style.TextAppearance_Medium)
            }

            val deleteBtn = Button(this).apply {
                text = "✕"
                setOnClickListener {
                    dictManager.removeEntry(entry.id)
                    refreshDictionaryList()
                    Toast.makeText(this@SettingsActivity, getString(R.string.dict_removed, entry.word), Toast.LENGTH_SHORT).show()
                }
            }

            row.addView(label)
            row.addView(deleteBtn)
            dictionaryList.addView(row)
        }
    }

    private fun showAddWordDialog() {
        val dialogLayout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 24, 48, 0)
        }

        val etWord = EditText(this).apply {
            hint = getString(R.string.dict_word_hint)
        }
        val etCorrectSpelling = EditText(this).apply {
            hint = getString(R.string.dict_correct_hint)
        }
        val etCategory = EditText(this).apply {
            hint = getString(R.string.dict_category_hint)
        }

        dialogLayout.addView(etWord)
        dialogLayout.addView(etCorrectSpelling)
        dialogLayout.addView(etCategory)

        AlertDialog.Builder(this)
            .setTitle(getString(R.string.dict_add_title))
            .setView(dialogLayout)
            .setPositiveButton(getString(R.string.dict_add)) { _, _ ->
                val word = etWord.text.toString().trim()
                val correctSpelling = etCorrectSpelling.text.toString().trim()
                val category = etCategory.text.toString().trim()

                if (word.isBlank() || correctSpelling.isBlank()) {
                    Toast.makeText(this, getString(R.string.dict_required), Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }

                DictionaryManager(this).addEntry(word, correctSpelling, category)
                refreshDictionaryList()
                Toast.makeText(this, getString(R.string.dict_added, word, correctSpelling), Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton(getString(R.string.dict_cancel), null)
            .show()
    }

    // ---------- Dictionary import/export ----------

    /**
     * Launch SAF ACTION_CREATE_DOCUMENT to export dictionary as JSON.
     */
    private fun launchDictionaryExport() {
        val dictManager = DictionaryManager(this)
        if (dictManager.count() == 0) {
            Toast.makeText(this, getString(R.string.dict_no_entries_to_export), Toast.LENGTH_SHORT).show()
            return
        }

        val dateFormat = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.getDefault())
        val fileName = "dictionary_export_${dateFormat.format(Date())}.json"

        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/json"
            putExtra(Intent.EXTRA_TITLE, fileName)
        }
        @Suppress("DEPRECATION")
        startActivityForResult(intent, REQUEST_DICT_EXPORT)
    }

    /**
     * Launch SAF ACTION_OPEN_DOCUMENT to pick a JSON file for dictionary import.
     */
    private fun launchDictionaryImport() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/json"
        }
        @Suppress("DEPRECATION")
        startActivityForResult(intent, REQUEST_DICT_IMPORT)
    }

    @Deprecated("Needed for SAF document result")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)

        if (resultCode != RESULT_OK || data == null) return

        when (requestCode) {
            REQUEST_DICT_EXPORT -> {
                val uri = data.data ?: return
                writeDictionaryExport(uri)
            }
            REQUEST_DICT_IMPORT -> {
                val uri = data.data ?: return
                readDictionaryImport(uri)
            }
        }
    }

    /**
     * Write dictionary JSON to the SAF-provided [uri].
     */
    private fun writeDictionaryExport(uri: Uri) {
        lifecycleScope.launch {
            val dictManager = DictionaryManager(this@SettingsActivity)
            val json = dictManager.exportToJson()

            withContext(Dispatchers.IO) {
                try {
                    contentResolver.openOutputStream(uri)?.use { os ->
                        OutputStreamWriter(os, "UTF-8").use { writer ->
                            writer.write(json)
                        }
                    }
                } catch (e: Exception) {
                    withContext(Dispatchers.Main) {
                        Toast.makeText(
                            this@SettingsActivity,
                            getString(R.string.dict_export_failed, e.message),
                            Toast.LENGTH_LONG
                        ).show()
                    }
                    return@withContext
                }
            }

            withContext(Dispatchers.Main) {
                Toast.makeText(
                    this@SettingsActivity,
                    getString(R.string.dict_export_success),
                    Toast.LENGTH_SHORT
                ).show()
            }
        }
    }

    /**
     * Read a JSON file from the SAF-provided [uri] and import into dictionary.
     */
    private fun readDictionaryImport(uri: Uri) {
        lifecycleScope.launch {
            val json = withContext(Dispatchers.IO) {
                try {
                    contentResolver.openInputStream(uri)?.use { is_ ->
                        BufferedReader(InputStreamReader(is_, "UTF-8")).use { reader ->
                            reader.readText()
                        }
                    }
                } catch (e: Exception) {
                    withContext(Dispatchers.Main) {
                        Toast.makeText(
                            this@SettingsActivity,
                            getString(R.string.dict_import_failed, e.message),
                            Toast.LENGTH_LONG
                        ).show()
                    }
                    return@withContext null
                }
            }

            if (json == null) return@launch

            withContext(Dispatchers.Main) {
                try {
                    val dictManager = DictionaryManager(this@SettingsActivity)
                    val countBefore = dictManager.count()
                    dictManager.importFromJson(json)
                    val countAfter = dictManager.count()
                    val added = countAfter - countBefore
                    refreshDictionaryList()
                    Toast.makeText(
                        this@SettingsActivity,
                        getString(R.string.dict_import_success, added, countAfter),
                        Toast.LENGTH_SHORT
                    ).show()
                } catch (e: Exception) {
                    Toast.makeText(
                        this@SettingsActivity,
                        getString(R.string.dict_import_invalid_json),
                        Toast.LENGTH_LONG
                    ).show()
                }
            }
        }
    }

    override fun onPause() {
        super.onPause()
        saveSettings()
    }
}
