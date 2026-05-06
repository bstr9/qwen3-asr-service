package com.qwen3.asr.typeless

import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Records audio from the device microphone in 16 kHz, mono, 16-bit PCM format.
 * Provides start()/stop() controls and converts PCM data to WAV format for ASR upload.
 */
class AudioRecorder {

    companion object {
        const val SAMPLE_RATE = 16000
        const val CHANNEL_CONFIG = AudioFormat.CHANNEL_IN_MONO
        const val AUDIO_FORMAT = AudioFormat.ENCODING_PCM_16BIT
        const val CHUNK_SIZE_MS = 30  // Silero VAD expects 30ms chunks
        const val CHUNK_SIZE_SAMPLES = SAMPLE_RATE * CHUNK_SIZE_MS / 1000 // 480 samples
        const val CHUNK_SIZE_BYTES = CHUNK_SIZE_SAMPLES * 2 // 16-bit = 2 bytes per sample

        /**
         * Convert PCM ShortArray to WAV byte array with proper header.
         */
        fun pcmToWav(samples: ShortArray, sampleRate: Int): ByteArray {
            val numChannels = 1
            val bitsPerSample = 16
            val byteRate = sampleRate * numChannels * bitsPerSample / 8
            val blockAlign = numChannels * bitsPerSample / 8
            val dataSize = samples.size * blockAlign
            val totalSize = 36 + dataSize

            val buffer = ByteBuffer.allocate(totalSize + 8)
            buffer.order(ByteOrder.LITTLE_ENDIAN)

            // RIFF header
            buffer.put("RIFF".toByteArray())
            buffer.putInt(totalSize)
            buffer.put("WAVE".toByteArray())

            // fmt chunk
            buffer.put("fmt ".toByteArray())
            buffer.putInt(16) // chunk size
            buffer.putShort(1) // PCM format
            buffer.putShort(numChannels.toShort())
            buffer.putInt(sampleRate)
            buffer.putInt(byteRate)
            buffer.putShort(blockAlign.toShort())
            buffer.putShort(bitsPerSample.toShort())

            // data chunk
            buffer.put("data".toByteArray())
            buffer.putInt(dataSize)

            for (sample in samples) {
                buffer.putShort(sample)
            }

            return buffer.array()
        }
    }

    private var audioRecord: AudioRecord? = null
    private var isRecording = false
    private val pcmBuffer = ByteArrayOutputStream()
    private var startTimeMs: Long = 0L

    // Callback for VAD processing — called on each audio chunk
    var onChunkAvailable: ((ShortArray) -> Unit)? = null

    /**
     * Start recording audio from the microphone.
     * Must be called from a thread that has audio permission already granted.
     */
    fun start() {
        if (isRecording) return

        val minBufferSize = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_CONFIG, AUDIO_FORMAT)
        val bufferSize = maxOf(minBufferSize, CHUNK_SIZE_BYTES * 4)

        audioRecord = AudioRecord(
            MediaRecorder.AudioSource.VOICE_RECOGNITION,
            SAMPLE_RATE,
            CHANNEL_CONFIG,
            AUDIO_FORMAT,
            bufferSize
        )

        if (audioRecord?.state != AudioRecord.STATE_INITIALIZED) {
            throw IllegalStateException("AudioRecord not initialized. Check microphone permission.")
        }

        pcmBuffer.reset()
        audioRecord?.startRecording()
        isRecording = true
        startTimeMs = System.currentTimeMillis()

        // Start reading thread
        Thread {
            val chunkBuffer = ShortArray(CHUNK_SIZE_SAMPLES)
            while (isRecording) {
                val read = audioRecord?.read(chunkBuffer, 0, CHUNK_SIZE_SAMPLES)
                    ?: break

                if (read > 0) {
                    // Store PCM data
                    val byteChunk = ShortArray(read)
                    System.arraycopy(chunkBuffer, 0, byteChunk, 0, read)
                    synchronized(pcmBuffer) {
                        for (sample in byteChunk) {
                            pcmBuffer.write(sample.toInt() and 0xFF)
                            pcmBuffer.write((sample.toInt() shr 8) and 0xFF)
                        }
                    }

                    // Notify VAD listener
                    onChunkAvailable?.invoke(byteChunk)
                }
            }
        }.start()
    }

    /**
     * Stop recording and return the raw PCM data as a ShortArray.
     * @return Pair of (pcmShortArray, durationSeconds)
     */
    fun stop(): Pair<ShortArray, Float> {
        isRecording = false

        audioRecord?.apply {
            try {
                stop()
            } catch (_: IllegalStateException) {
                // Already stopped
            }
            release()
        }
        audioRecord = null

        val durationSecs = (System.currentTimeMillis() - startTimeMs) / 1000f

        val pcmBytes = synchronized(pcmBuffer) { pcmBuffer.toByteArray() }
        val samples = ShortArray(pcmBytes.size / 2)
        ByteBuffer.wrap(pcmBytes).order(ByteOrder.LITTLE_ENDIAN).asShortBuffer().get(samples)

        return Pair(samples, durationSecs)
    }

    /**
     * Stop recording and return WAV-formatted byte array ready for ASR upload.
     */
    fun stopAsWav(): Pair<ByteArray, Float> {
        val (samples, duration) = stop()
        val wavData = pcmToWav(samples, SAMPLE_RATE)
        return Pair(wavData, duration)
    }

    fun isRecording(): Boolean = isRecording

    /**
     * Release all resources.
     */
    fun release() {
        isRecording = false
        audioRecord?.apply {
            try { stop() } catch (_: Exception) {}
            release()
        }
        audioRecord = null
    }

}
