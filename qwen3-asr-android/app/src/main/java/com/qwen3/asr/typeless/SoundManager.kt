package com.qwen3.asr.typeless

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import kotlin.math.PI
import kotlin.math.sin

/**
 * Manages sound effects for recording state feedback.
 *
 * Generates beep tones programmatically using [AudioTrack] with 16-bit PCM sine waves.
 * No external sound files or third-party libraries required.
 *
 * Tones:
 *  - Start: 800 Hz, 300 ms (high beep)
 *  - Stop:  400 Hz, 300 ms (low beep)
 *
 * Respects the `play_sounds` SharedPreferences setting (default: true).
 */
object SoundManager {

    private const val SAMPLE_RATE = 44100
    private const val FADE_MS = 5L
    private const val AMPLITUDE = 16000.0 // Leave headroom (max 32767)

    /** Generate a 16-bit mono PCM sine wave with fade-in/fade-out envelope. */
    private fun generateTone(frequency: Int, durationMs: Long): ShortArray {
        val numSamples = (SAMPLE_RATE.toLong() * durationMs / 1000).toInt()
        val fadeSamples = (SAMPLE_RATE.toDouble() * FADE_MS / 1000.0).toInt()
        val samples = ShortArray(numSamples)

        for (i in 0 until numSamples) {
            val t = i.toDouble() / SAMPLE_RATE
            val raw = AMPLITUDE * sin(2.0 * PI * frequency * t)

            val envelope = when {
                i < fadeSamples -> i.toDouble() / fadeSamples
                i > numSamples - fadeSamples -> (numSamples - i).toDouble() / fadeSamples
                else -> 1.0
            }

            samples[i] = (raw * envelope).toInt().coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()
        }

        return samples
    }

    /** Play a tone asynchronously on a background thread. */
    private fun playTone(samples: ShortArray) {
        val bufferSize = samples.size * 2 // 16-bit = 2 bytes per sample

        val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION_EVENT)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()

        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(SAMPLE_RATE)
            .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
            .build()

        val track = AudioTrack.Builder()
            .setAudioAttributes(attributes)
            .setAudioFormat(format)
            .setBufferSizeInBytes(bufferSize)
            .setTransferMode(AudioTrack.MODE_STATIC)
            .build()

        track.write(samples, 0, samples.size)
        track.play()

        // Release after playback completes (approximate)
        Thread {
            val durationMs = samples.size.toLong() * 1000 / SAMPLE_RATE
            Thread.sleep(durationMs + 100)
            try {
                track.stop()
                track.release()
            } catch (_: Exception) {}
        }.start()
    }

    private fun shouldPlay(context: Context): Boolean {
        val prefs = context.getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
        return prefs.getBoolean("play_sounds", true)
    }

    /** Play the start-recording beep (800 Hz, 300 ms). */
    fun playStartSound(context: Context) {
        if (!shouldPlay(context)) return
        val samples = generateTone(800, 300)
        playTone(samples)
    }

    /** Play the stop-recording beep (400 Hz, 300 ms). */
    fun playStopSound(context: Context) {
        if (!shouldPlay(context)) return
        val samples = generateTone(400, 300)
        playTone(samples)
    }


}
