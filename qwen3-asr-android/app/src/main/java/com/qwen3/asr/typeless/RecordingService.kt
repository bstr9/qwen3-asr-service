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
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import androidx.core.app.NotificationCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import android.util.Log

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
 *  - ACTION_RECORDING_DURATION    (extra: EXTRA_DURATION_SECONDS)
 *  - ACTION_RECORDING_CANCELLED
 *  - ACTION_TOO_SHORT
 *  - ACTION_MAX_DURATION_REACHED
 */
class RecordingService : LifecycleService() {

    companion object {
        const val CHANNEL_ID = "recording_channel"
        const val NOTIFICATION_ID = 1001

        // Intent actions
        const val ACTION_START_RECORDING = "com.qwen3.asr.typeless.START_RECORDING"
        const val ACTION_STOP_RECORDING = "com.qwen3.asr.typeless.STOP_RECORDING"
        const val ACTION_CANCEL_RECORDING = "com.qwen3.asr.typeless.CANCEL_RECORDING"

        // Broadcast actions
        const val ACTION_RECORDING_STARTED = "com.qwen3.asr.typeless.RECORDING_STARTED"
        const val ACTION_RECORDING_STOPPED = "com.qwen3.asr.typeless.RECORDING_STOPPED"
        const val ACTION_TRANSCRIPTION_RESULT = "com.qwen3.asr.typeless.TRANSCRIPTION_RESULT"
        const val ACTION_TRANSCRIPTION_ERROR = "com.qwen3.asr.typeless.TRANSCRIPTION_ERROR"
        const val ACTION_STATE_CHANGED = "com.qwen3.asr.typeless.STATE_CHANGED"
        const val ACTION_RECORDING_DURATION = "com.qwen3.asr.typeless.RECORDING_DURATION"
        const val ACTION_RECORDING_CANCELLED = "com.qwen3.asr.typeless.RECORDING_CANCELLED"
        const val ACTION_TOO_SHORT = "com.qwen3.asr.typeless.TOO_SHORT"
        const val ACTION_MAX_DURATION_REACHED = "com.qwen3.asr.typeless.MAX_DURATION_REACHED"

        // Broadcast extras
        const val EXTRA_TEXT = "text"
        const val EXTRA_ERROR = "error"
        const val EXTRA_STATE = "state"
        const val EXTRA_DURATION_SECONDS = "duration_seconds"

        // Recording modes
        const val MODE_PTT = "ptt"
        const val MODE_HANDSFREE = "handsfree"

        // Defaults
        private const val DEFAULT_MAX_RECORDING_DURATION = 60 // seconds
        private const val MIN_RECORDING_DURATION = 0.5f // seconds — below this, discard
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

    // Recording timing
    private var recordingStartTime: Long = 0L
    private val durationHandler = Handler(Looper.getMainLooper())
    private val maxDurationHandler = Handler(Looper.getMainLooper())
    private var durationUpdateRunnable: Runnable? = null
    private var maxDurationRunnable: Runnable? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)

        when (intent?.action) {
            ACTION_START_RECORDING -> startRecording()
            ACTION_STOP_RECORDING -> stopRecording()
            ACTION_CANCEL_RECORDING -> cancelRecording()
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
        val notification = buildNotification(getString(R.string.notification_recording))
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
            val vad = vadDetector
            if (vad != null) {
                audioRecorder?.onChunkAvailable = { chunk ->
                    vad.processChunk(chunk)
                    if (vad.isSilenceDurationExceeded()) {
                        // Auto-stop on silence
                        stopRecording()
                    }
                }
            }
        }

        try {
            audioRecorder?.start()
            currentState = State.RECORDING
            recordingStartTime = System.currentTimeMillis()
            broadcastState(currentState)
            broadcastAction(ACTION_RECORDING_STARTED)
            updateNotification(
                if (currentMode == MODE_HANDSFREE) getString(R.string.status_recording_handsfree)
                else getString(R.string.status_recording_ptt)
            )

            // Play start sound
            SoundManager.playStartSound(this)

            // Start duration update timer (every 200ms)
            startDurationUpdates()

            // Start max duration timer
            startMaxDurationTimer()

        } catch (e: Exception) {
            stopSelf()
            broadcastError(getString(R.string.error_recording_start_failed, e.message))
        }
    }

    private fun stopRecording() {
        if (currentState != State.RECORDING) return

        val recorder = audioRecorder ?: return
        audioRecorder = null

        // Stop timers
        stopDurationUpdates()
        stopMaxDurationTimer()

        val (wavData, duration) = recorder.stopAsWav()
        vadDetector?.release()
        vadDetector = null

        // Play stop sound
        SoundManager.playStopSound(this)

        // Check if recording was too short (< 0.5s)
        if (duration < MIN_RECORDING_DURATION) {
            currentState = State.IDLE
            broadcastState(currentState)
            broadcastAction(ACTION_TOO_SHORT)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return
        }

        currentState = State.PROCESSING
        broadcastState(currentState)
        broadcastAction(ACTION_RECORDING_STOPPED)
        updateNotification(getString(R.string.notification_processing))

        // Process the recording
        lifecycleScope.launch {
            processRecording(wavData, duration)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    /**
     * Cancel recording without submitting to ASR.
     * Discards the recorded audio and broadcasts cancellation.
     */
    private fun cancelRecording() {
        if (currentState != State.RECORDING) return

        val recorder = audioRecorder
        audioRecorder = null

        // Stop timers
        stopDurationUpdates()
        stopMaxDurationTimer()

        // Release recorder without processing
        recorder?.let {
            it.release()
        }
        vadDetector?.release()
        vadDetector = null

        // Play stop sound
        SoundManager.playStopSound(this)

        currentState = State.IDLE
        broadcastState(currentState)
        broadcastAction(ACTION_RECORDING_CANCELLED)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    // ---------- Duration tracking ----------

    private fun startDurationUpdates() {
        durationUpdateRunnable = object : Runnable {
            override fun run() {
                if (currentState != State.RECORDING) return
                val elapsedSec = (System.currentTimeMillis() - recordingStartTime) / 1000.0f
                val intent = Intent(ACTION_RECORDING_DURATION)
                intent.putExtra(EXTRA_DURATION_SECONDS, elapsedSec)
                sendBroadcast(intent)
                durationHandler.postDelayed(this, 200)
            }
        }
        durationHandler.post(durationUpdateRunnable!!)
    }

    private fun stopDurationUpdates() {
        durationUpdateRunnable?.let { durationHandler.removeCallbacks(it) }
        durationUpdateRunnable = null
    }

    // ---------- Max duration timer ----------

    private fun startMaxDurationTimer() {
        val maxDurationSec = prefs.getInt("max_recording_duration", DEFAULT_MAX_RECORDING_DURATION)
        if (maxDurationSec <= 0) return // 0 = no limit

        maxDurationRunnable = Runnable {
            if (currentState == State.RECORDING) {
                broadcastAction(ACTION_MAX_DURATION_REACHED)
                stopRecording()
            }
        }
        maxDurationHandler.postDelayed(maxDurationRunnable!!, maxDurationSec * 1000L)
    }

    private fun stopMaxDurationTimer() {
        maxDurationRunnable?.let { maxDurationHandler.removeCallbacks(it) }
        maxDurationRunnable = null
    }

    private suspend fun processRecording(wavData: ByteArray, duration: Float) {
        val asrClient = App.getInstance().asrClient

        var result = asrClient.transcribe(wavData)
            .getOrElse { e ->
                withContext(Dispatchers.Main) {
                    currentState = State.IDLE
                    broadcastState(currentState)
                    broadcastError(e.message ?: getString(R.string.error_transcription_failed))
                }
                return
            }

        // --- Post-processing pipeline ---
        val rawText = result

        if (prefs.getBoolean("post_processing", true)) {
            val removeFillers = prefs.getBoolean("remove_fillers", true)
            val removeRepetitions = prefs.getBoolean("remove_repetitions", true)
            val autoFormat = prefs.getBoolean("auto_format", true)
            result = PostProcessor.postprocess(result, removeFillers, removeRepetitions, autoFormat)
        }

        // --- LLM post-processing ---
        val llmEnabled = prefs.getBoolean("llm_enabled", false)
        val llmUrl = prefs.getString("llm_url", "") ?: ""
        val llmModel = prefs.getString("llm_model", "") ?: ""
        val llmApiKey = prefs.getString("llm_api_key", "") ?: ""
        val customPrompt = prefs.getString("custom_prompt", "") ?: ""

        if (llmEnabled && llmUrl.isNotBlank() && llmModel.isNotBlank()) {
            try {
                val dictionaryHint = DictionaryManager(this@RecordingService).formatForPrompt()
                result = withContext(Dispatchers.IO) {
                    PostProcessor.llmPostprocess(result, llmUrl, llmModel, llmApiKey, customPrompt, dictionaryHint)
                }
            } catch (e: Exception) {
                Log.w("RecordingService", "LLM post-processing failed", e)
            }
        }

        // Save to history
        val mode = currentMode
        withContext(Dispatchers.IO) {
            val db = HistoryDatabase.getInstance(this@RecordingService)
            val entry = HistoryEntry(
                text = result,
                rawText = rawText,
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
            getString(R.string.notification_channel_name),
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = getString(R.string.notification_channel_desc)
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

        val cancelIntent = PendingIntent.getService(
            this, 2,
            Intent(this, RecordingService::class.java).apply {
                action = ACTION_CANCEL_RECORDING
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.notification_title))
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_mic)
            .setContentIntent(pendingIntent)
            .addAction(android.R.drawable.ic_media_pause, getString(R.string.notification_stop), stopIntent)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, getString(R.string.notification_cancel), cancelIntent)
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
        stopDurationUpdates()
        stopMaxDurationTimer()
        audioRecorder?.release()
        audioRecorder = null
        vadDetector?.release()
        vadDetector = null
    }
}
