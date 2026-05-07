package com.qwen3.asr.typeless

import android.content.Context
import android.content.SharedPreferences
import ai.onnxruntime.OnnxTensor
import ai.onnxruntime.OrtEnvironment
import ai.onnxruntime.OrtSession
import java.nio.LongBuffer

/**
 * Silero VAD (Voice Activity Detection) wrapper using ONNX Runtime.
 *
 * Loads the silero_vad.onnx model from assets and processes audio chunks
 * to determine speech probability. Used in Hands-free mode to auto-stop
 * recording when silence is detected.
 *
 * The Silero VAD model expects:
 *  - Sample rate: 16000 Hz
 *  - Chunk size: 512, 1024, or 1536 samples (32/64/96 ms)
 *  - Input: float tensor of shape [1, chunk_size]
 *
 * Model file should be placed at: assets/silero_vad.onnx
 * Download from: https://github.com/snakers4/silero-vad
 */
class VadDetector(private val context: Context) {

    companion object {
        const val MODEL_FILENAME = "silero_vad.onnx"
        const val SAMPLE_RATE = 16000L
        const val CHUNK_SIZE = 512  // 32ms chunks for Silero VAD
        const val DEFAULT_THRESHOLD = 0.5f
        const val DEFAULT_SILENCE_DURATION_SEC = 2.0f
    }

    private val prefs: SharedPreferences by lazy {
        context.getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
    }

    private var env: OrtEnvironment? = null
    private var session: OrtSession? = null

    // State tensors for the recurrent model
    private var hTensor: OnnxTensor? = null
    private var cTensor: OnnxTensor? = null

    private var silenceStartMs: Long = 0L
    private var isSpeechDetected = false
    private var lastProbability: Float = 0f

    /**
     * Initialize the VAD model. Must be called before processing.
     */
    fun initialize() {
        if (session != null) return

        env = OrtEnvironment.getEnvironment()

        // Copy model from assets to cache dir for ONNX Runtime
        val modelFile = copyModelToCache()

        val sessionOptions = OrtSession.SessionOptions()
        sessionOptions.setOptimizationLevel(OrtSession.SessionOptions.OptLevel.ALL_OPT)
        sessionOptions.setInterOpNumThreads(1)
        sessionOptions.setIntraOpNumThreads(1)

        session = env?.createSession(modelFile.absolutePath, sessionOptions)

        resetState()
    }

    private fun copyModelToCache(): java.io.File {
        val cacheFile = java.io.File(context.cacheDir, MODEL_FILENAME)
        if (!cacheFile.exists()) {
            context.assets.open(MODEL_FILENAME).use { input ->
                java.io.FileOutputStream(cacheFile).use { output ->
                    input.copyTo(output)
                }
            }
        }
        return cacheFile
    }

    /**
     * Reset internal state. Call between recordings.
     */
    fun resetState() {
        // Initialize hidden state tensors (2x1x64 zeros)
        val hShape = longArrayOf(2, 1, 64)
        val hData = FloatArray(2 * 1 * 64) { 0f }
        val cData = FloatArray(2 * 1 * 64) { 0f }

        hTensor?.close()
        cTensor?.close()

        val hBuffer = java.nio.FloatBuffer.wrap(hData)
        val cBuffer = java.nio.FloatBuffer.wrap(cData)

        hTensor = OnnxTensor.createTensor(env, hBuffer, hShape)
        cTensor = OnnxTensor.createTensor(env, cBuffer, hShape)

        silenceStartMs = 0L
        isSpeechDetected = false
        lastProbability = 0f
    }

    /**
     * Process a ShortArray chunk and return the speech probability.
     *
     * @param chunk Audio samples (should be CHUNK_SIZE length, but handles any size)
     * @return Speech probability [0.0, 1.0]
     */
    fun processChunk(chunk: ShortArray): Float {
        val sess = session ?: return 0f
        val environment = env ?: return 0f

        // Process in CHUNK_SIZE windows
        var totalProb = 0f
        var count = 0

        var offset = 0
        while (offset + CHUNK_SIZE <= chunk.size) {
            val subChunk = chunk.copyOfRange(offset, offset + CHUNK_SIZE)
            val prob = runInference(environment, sess, subChunk)
            totalProb += prob
            count++
            offset += CHUNK_SIZE
        }

        // Handle remaining samples
        if (offset < chunk.size && count == 0) {
            // Pad to CHUNK_SIZE
            val padded = ShortArray(CHUNK_SIZE)
            System.arraycopy(chunk, 0, padded, 0, chunk.size)
            val prob = runInference(environment, sess, padded)
            totalProb += prob
            count++
        }

        lastProbability = if (count > 0) totalProb / count else 0f
        return lastProbability
    }

    private fun runInference(env: OrtEnvironment, session: OrtSession, chunk: ShortArray): Float {
        // Convert short samples to float [-1.0, 1.0]
        val floatSamples = FloatArray(chunk.size) { i ->
            chunk[i].toFloat() / 32768.0f
        }

        val inputBuffer = java.nio.FloatBuffer.wrap(floatSamples)
        val inputTensor = OnnxTensor.createTensor(env, inputBuffer, longArrayOf(1, chunk.size.toLong()))
        val srBuffer = LongBuffer.wrap(longArrayOf(SAMPLE_RATE))
        val srTensor = OnnxTensor.createTensor(env, srBuffer, longArrayOf(1))

        val currentH = hTensor ?: return 0f
        val currentC = cTensor ?: return 0f

        val inputs = mapOf(
            "input" to inputTensor,
            "sr" to srTensor,
            "h" to currentH,
            "c" to currentC
        )

        val output = session.run(inputs)

        // Extract probability
        val probTensor = output.get(0) as OnnxTensor
        val prob = probTensor.floatBuffer.get()

        // Update hidden states
        val newH = output.get(2) as OnnxTensor
        val newC = output.get(3) as OnnxTensor

        hTensor?.close()
        cTensor?.close()
        hTensor = newH
        cTensor = newC

        inputTensor.close()
        srTensor.close()
        probTensor.close()
        // Don't close newH/newC since we're holding references

        return prob
    }

    /**
     * Check if silence duration has exceeded the threshold for auto-stop.
     * Call this after processChunk() to track silence.
     *
     * @return true if silence duration exceeded the configured threshold
     */
    fun isSilenceDurationExceeded(): Boolean {
        val threshold = getThreshold()
        val silenceDurationSec = getSilenceDurationSec()

        if (lastProbability < threshold) {
            if (silenceStartMs == 0L) {
                silenceStartMs = System.currentTimeMillis()
            }
            val elapsed = (System.currentTimeMillis() - silenceStartMs) / 1000f
            return elapsed >= silenceDurationSec && isSpeechDetected
        } else {
            // Speech detected, reset silence timer
            silenceStartMs = 0L
            isSpeechDetected = true
            return false
        }
    }

    fun getThreshold(): Float =
        prefs.getFloat("vad_threshold", DEFAULT_THRESHOLD)

    fun getSilenceDurationSec(): Float =
        prefs.getFloat("silence_duration", DEFAULT_SILENCE_DURATION_SEC)

    /**
     * Release all ONNX resources.
     */
    fun release() {
        hTensor?.close()
        cTensor?.close()
        hTensor = null
        cTensor = null
        session?.close()
        session = null
        // Don't close env — it's a singleton
    }
}
