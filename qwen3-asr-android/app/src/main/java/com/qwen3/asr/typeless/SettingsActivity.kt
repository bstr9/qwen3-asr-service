package com.qwen3.asr.typeless

import android.content.Context
import android.content.SharedPreferences
import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.Switch
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.appbar.MaterialToolbar

/**
 * Settings screen for ASR service configuration.
 *
 * Settings:
 *  - ASR URL (EditText, default http://192.168.1.100:8765)
 *  - API Key (EditText, password type)
 *  - Default mode (dropdown: PTT / Hands-free)
 *  - VAD threshold (SeekBar 0.0–1.0)
 *  - Silence duration (EditText, seconds)
 *  - Post-processing toggle
 *
 * All settings saved to SharedPreferences.
 */
class SettingsActivity : AppCompatActivity() {

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
    private lateinit var switchPostProcessing: Switch

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

        toolbar.setNavigationOnClickListener { finish() }

        // Mode dropdown
        val modes = arrayOf("PTT (Push-to-Talk)", "Hands-free (VAD auto-stop)")
        val adapter = ArrayAdapter(this, android.R.layout.simple_spinner_dropdown_item, modes)
        spinnerMode.adapter = adapter

        // VAD threshold SeekBar: 0-100 mapped to 0.0-1.0
        seekVadThreshold.max = 100
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

            apply()
        }
    }

    override fun onPause() {
        super.onPause()
        saveSettings()
    }
}
