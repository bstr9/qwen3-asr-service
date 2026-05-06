package com.qwen3.asr.typeless

import android.Manifest
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.google.android.material.chip.Chip
import com.google.android.material.floatingactionbutton.FloatingActionButton
import com.google.android.material.bottomnavigation.BottomNavigationView

/**
 * Main activity with Material Design 3 UI.
 *
 * Features:
 *  - Large floating action button (mic icon) — tap to start/stop recording
 *  - Status text showing current state (Idle / Recording / Processing / Pasting)
 *  - Mode toggle chip (PTT / Hands-free)
 *  - Bottom navigation: Record | History | Settings
 *  - Last transcription result display
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val PERMISSION_REQUEST_AUDIO = 1001
        private const val PERMISSION_REQUEST_NOTIFICATIONS = 1002
    }

    private val prefs: SharedPreferences by lazy {
        getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
    }

    // Views
    private lateinit var fabRecord: FloatingActionButton
    private lateinit var tvStatus: TextView
    private lateinit var tvResult: TextView
    private lateinit var chipPtt: Chip
    private lateinit var chipHandsfree: Chip
    private lateinit var bottomNav: BottomNavigationView

    private var currentState = RecordingService.State.IDLE
    private var currentMode = RecordingService.MODE_PTT

    // Broadcast receiver for service events
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            when (intent?.action) {
                RecordingService.ACTION_STATE_CHANGED -> {
                    val stateName = intent.getStringExtra(RecordingService.EXTRA_STATE) ?: return
                    currentState = RecordingService.State.valueOf(stateName)
                    updateUI()
                }
                RecordingService.ACTION_TRANSCRIPTION_RESULT -> {
                    val text = intent.getStringExtra(RecordingService.EXTRA_TEXT) ?: ""
                    val duration = intent.getFloatExtra(RecordingService.EXTRA_DURATION, 0f)
                    tvResult.text = text
                    Toast.makeText(this@MainActivity, "Copied to clipboard", Toast.LENGTH_SHORT).show()
                }
                RecordingService.ACTION_TRANSCRIPTION_ERROR -> {
                    val error = intent.getStringExtra(RecordingService.EXTRA_ERROR) ?: "Unknown error"
                    Toast.makeText(this@MainActivity, "Error: $error", Toast.LENGTH_LONG).show()
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        initViews()
        setupListeners()
        updateUI()
    }

    override fun onResume() {
        super.onResume()

        // Register broadcast receiver
        val filter = IntentFilter().apply {
            addAction(RecordingService.ACTION_STATE_CHANGED)
            addAction(RecordingService.ACTION_TRANSCRIPTION_RESULT)
            addAction(RecordingService.ACTION_TRANSCRIPTION_ERROR)
            addAction(RecordingService.ACTION_RECORDING_STARTED)
            addAction(RecordingService.ACTION_RECORDING_STOPPED)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(receiver, filter)
        }

        // Refresh mode from prefs
        currentMode = prefs.getString("default_mode", RecordingService.MODE_PTT)
            ?: RecordingService.MODE_PTT
        updateModeChips()
    }

    override fun onPause() {
        super.onPause()
        try {
            unregisterReceiver(receiver)
        } catch (_: Exception) {}
    }

    private fun initViews() {
        fabRecord = findViewById(R.id.fab_record)
        tvStatus = findViewById(R.id.tv_status)
        tvResult = findViewById(R.id.tv_result)
        chipPtt = findViewById(R.id.chip_ptt)
        chipHandsfree = findViewById(R.id.chip_handsfree)
        bottomNav = findViewById(R.id.bottom_nav)

        currentMode = prefs.getString("default_mode", RecordingService.MODE_PTT)
            ?: RecordingService.MODE_PTT
        updateModeChips()
    }

    private fun setupListeners() {
        fabRecord.setOnClickListener {
            onRecordButtonClicked()
        }

        chipPtt.setOnClickListener {
            currentMode = RecordingService.MODE_PTT
            prefs.edit().putString("default_mode", currentMode).apply()
            updateModeChips()
        }

        chipHandsfree.setOnClickListener {
            currentMode = RecordingService.MODE_HANDSFREE
            prefs.edit().putString("default_mode", currentMode).apply()
            updateModeChips()
        }

        bottomNav.setOnItemSelectedListener { item ->
            when (item.itemId) {
                R.id.nav_record -> true // Already here
                R.id.nav_history -> {
                    startActivity(Intent(this, HistoryActivity::class.java))
                    true
                }
                R.id.nav_settings -> {
                    startActivity(Intent(this, SettingsActivity::class.java))
                    true
                }
                else -> false
            }
        }

        // Default to record tab
        bottomNav.selectedItemId = R.id.nav_record
    }

    private fun onRecordButtonClicked() {
        when (currentState) {
            RecordingService.State.IDLE, RecordingService.State.PASTING -> {
                // Start recording
                if (!checkAudioPermission()) {
                    requestAudioPermission()
                    return
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    if (!checkNotificationPermission()) {
                        requestNotificationPermission()
                        // Continue anyway — notifications aren't critical
                    }
                }
                startRecordingService()
            }
            RecordingService.State.RECORDING -> {
                // Stop recording (PTT mode or manual stop)
                stopRecordingService()
            }
            RecordingService.State.PROCESSING -> {
                Toast.makeText(this, "Processing... please wait", Toast.LENGTH_SHORT).show()
            }
        }
    }

    private fun startRecordingService() {
        val intent = Intent(this, RecordingService::class.java).apply {
            action = RecordingService.ACTION_START_RECORDING
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun stopRecordingService() {
        val intent = Intent(this, RecordingService::class.java).apply {
            action = RecordingService.ACTION_STOP_RECORDING
        }
        startService(intent)
    }

    private fun updateUI() {
        val statusText: String
        val fabIcon: Int
        val bgColor: Int

        when (currentState) {
            RecordingService.State.IDLE -> {
                statusText = "Tap to record"
                fabIcon = R.drawable.ic_mic
                bgColor = ContextCompat.getColor(this, R.color.md_primary)
            }
            RecordingService.State.RECORDING -> {
                statusText = "Recording... (${currentMode})"
                fabIcon = android.R.drawable.ic_media_pause
                bgColor = ContextCompat.getColor(this, R.color.md_error)
            }
            RecordingService.State.PROCESSING -> {
                statusText = "Processing..."
                fabIcon = android.R.drawable.ic_popup_sync
                bgColor = ContextCompat.getColor(this, R.color.md_tertiary)
            }
            RecordingService.State.PASTING -> {
                statusText = "Pasting result..."
                fabIcon = R.drawable.ic_mic
                bgColor = ContextCompat.getColor(this, R.color.md_primary)
            }
        }

        tvStatus.text = statusText
        fabRecord.setImageResource(fabIcon)
        fabRecord.backgroundTintList = android.content.res.ColorStateList.valueOf(bgColor)
    }

    private fun updateModeChips() {
        chipPtt.isChecked = currentMode == RecordingService.MODE_PTT
        chipHandsfree.isChecked = currentMode == RecordingService.MODE_HANDSFREE
    }

    // ---------- Permission handling ----------

    private fun checkAudioPermission(): Boolean =
        ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
                PackageManager.PERMISSION_GRANTED

    private fun checkNotificationPermission(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
                    PackageManager.PERMISSION_GRANTED
        } else true

    private fun requestAudioPermission() {
        ActivityCompat.requestPermissions(
            this,
            arrayOf(Manifest.permission.RECORD_AUDIO),
            PERMISSION_REQUEST_AUDIO
        )
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ActivityCompat.requestPermissions(
                this,
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                PERMISSION_REQUEST_NOTIFICATIONS
            )
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        when (requestCode) {
            PERMISSION_REQUEST_AUDIO -> {
                if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
                    onRecordButtonClicked()
                } else {
                    Toast.makeText(this, "Microphone permission required", Toast.LENGTH_LONG).show()
                }
            }
        }
    }
}
