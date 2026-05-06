package com.qwen3.asr.typeless

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Foreground service that manages the audio recording lifecycle.
 *
 * Supports two modes:
 *  - PTT (Push-to-Talk): Start/stop via explicit intents
 *  - Hands-free: Auto-stops when VAD detects silence
 *
 * Communication with the UI happens via LocalBroadcast:
 *  - ACTION_RECORDING_STARTED
 *  - ACTION_RECORDING_STOPPED
 *  - ACTION_TRANSCRIPTION_RESULT  (extra: EXTRA_TEXT)
 *  - ACTION_TRANSCRIPTION_ERROR   (extra: EXTRA_ERROR)
 *  - ACTION_STATE_CHANGED         (extra: EXTRA_STATE)
 */
class RecordingService : LifecycleService() {

    companion object {
        const val CHANNEL_ID = "recording_channel"
        const val NOTIFICATION_ID = 1001

        // Intent actions
        const val ACTION_START_RECORDING = "com.qwen3.asr.typeless.START_RECORDING"
        const val ACTION_STOP_RECORDING = "com.qwen3.asr.typeless.STOP_RECORDING"

        // Broadcast actions
        const val ACTION_RECORDING_STARTED = "com.qwen3.asr.typeless.RECORDING_STARTED"
        const val ACTION_RECORDING_STOPPED = "com.qwen3.asr.typeless.RECORDING_STOPPED"
        const val ACTION_TRANSCRIPTION_RESULT = "com.qwen3.asr.typeless.TRANSCRIPTION_RESULT"
        const val ACTION_TRANSCRIPTION_ERROR = "com.qwen3.asr.typeless.TRANSCRIPTION_ERROR"
        const val ACTION_STATE_CHANGED = "com.qwen3.asr.typeless.STATE_CHANGED"

        // Broadcast extras
        const val EXTRA_TEXT = "text"
        const val EXTRA_ERROR = "error"
        const val EXTRA_STATE = "state"
        const val EXTRA_DURATION = "duration"

        // Recording modes
        const val MODE_PTT = "ptt"
        const val MODE_HANDSFREE = "handsfree"
    }

    enum class State {
        IDLE, RECORDING, PROCESSING, PASTING
    }

    private val prefs: SharedPreferences by lazy {
        getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
    }

    private var currentState = State.IDLE
    private var audioRecorder: AudioRecorder? = null
    private var vadDetector: VadDetector? = null
    private var currentMode: String = MODE_PTT

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)

        when (intent?.action) {
            ACTION_START_RECORDING -> startRecording()
            ACTION_STOP_RECORDING -> stopRecording()
        }

        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent): IBinder? {
        super.onBind(intent)
        return null
    }

    // ---------- Recording control ----------

    private fun startRecording() {
        if (currentState == State.RECORDING) return

        currentMode = prefs.getString("default_mode", MODE_PTT) ?: MODE_PTT

        // Start as foreground service with notification
        val notification = buildNotification("Recording audio...")
        startForeground(NOTIFICATION_ID, notification)

        // Initialize VAD for handsfree mode
        if (currentMode == MODE_HANDSFREE) {
            vadDetector = VadDetector(this).also {
                it.initialize()
                it.resetState()
            }
        }

        // Initialize and start audio recorder
        audioRecorder = AudioRecorder()

        if (currentMode == MODE_HANDSFREE) {
            audioRecorder?.onChunkAvailable = { chunk ->
                val vad = vadDetector ?: return@onChunkAvailable
                vad.processChunk(chunk)
                if (vad.isSilenceDurationExceeded()) {
                    // Auto-stop on silence
                    stopRecording()
                }
            }
        }

        try {
            audioRecorder?.start()
            currentState = State.RECORDING
            broadcastState(currentState)
            broadcastAction(ACTION_RECORDING_STARTED)
            updateNotification("Recording... (${currentMode})")
        } catch (e: Exception) {
            stopSelf()
            broadcastError("Failed to start recording: ${e.message}")
        }
    }

    private fun stopRecording() {
        if (currentState != State.RECORDING) return

        val recorder = audioRecorder ?: return
        audioRecorder = null

        val (wavData, duration) = recorder.stopAsWav()
        vadDetector?.release()
        vadDetector = null

        currentState = State.PROCESSING
        broadcastState(currentState)
        broadcastAction(ACTION_RECORDING_STOPPED)
        updateNotification("Processing transcription...")

        // Process the recording
        lifecycleScope.launch {
            val result = processRecording(wavData, duration)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private suspend fun processRecording(wavData: ByteArray, duration: Float) {
        val asrClient = App.getInstance().asrClient

        val result = asrClient.transcribe(wavData)
            .getOrElse { e ->
                withContext(Dispatchers.Main) {
                    currentState = State.IDLE
                    broadcastState(currentState)
                    broadcastError(e.message ?: "Transcription failed")
                }
                return
            }

        // Save to history
        val mode = currentMode
        withContext(Dispatchers.IO) {
            val db = HistoryDatabase.getInstance(this@RecordingService)
            val entry = HistoryEntry(
                text = result,
                rawText = result,
                timestamp = System.currentTimeMillis(),
                durationSecs = duration,
                mode = mode,
                language = ""
            )
            db.historyDao().insert(entry)
        }

        // Broadcast result
        withContext(Dispatchers.Main) {
            currentState = State.PASTING
            broadcastState(currentState)

            val intent = Intent(ACTION_TRANSCRIPTION_RESULT)
            intent.putExtra(EXTRA_TEXT, result)
            intent.putExtra(EXTRA_DURATION, duration)
            sendBroadcast(intent)

            // Copy to clipboard
            val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
            clipboard.setPrimaryClip(android.content.ClipData.newPlainText("ASR", result))

            // Reset state after a brief moment
            currentState = State.IDLE
            broadcastState(currentState)
        }
    }

    // ---------- Notification ----------

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Recording",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "Audio recording for ASR"
            setShowBadge(false)
        }

        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val stopIntent = PendingIntent.getService(
            this, 1,
            Intent(this, RecordingService::class.java).apply {
                action = ACTION_STOP_RECORDING
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Typeless ASR")
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_mic)
            .setContentIntent(pendingIntent)
            .addAction(android.R.drawable.ic_media_pause, "Stop", stopIntent)
            .setOngoing(true)
            .build()
    }

    private fun updateNotification(text: String) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(NOTIFICATION_ID, buildNotification(text))
    }

    // ---------- Broadcast helpers ----------

    private fun broadcastState(state: State) {
        val intent = Intent(ACTION_STATE_CHANGED)
        intent.putExtra(EXTRA_STATE, state.name)
        sendBroadcast(intent)
    }

    private fun broadcastAction(action: String) {
        sendBroadcast(Intent(action))
    }

    private fun broadcastError(message: String) {
        val intent = Intent(ACTION_TRANSCRIPTION_ERROR)
        intent.putExtra(EXTRA_ERROR, message)
        sendBroadcast(intent)
    }

    override fun onDestroy() {
        super.onDestroy()
        audioRecorder?.release()
        audioRecorder = null
        vadDetector?.release()
        vadDetector = null
    }
}
