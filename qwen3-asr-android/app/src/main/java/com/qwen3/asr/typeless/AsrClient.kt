package com.qwen3.asr.typeless

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.asRequestBody
import org.json.JSONObject
import java.io.File
import java.util.concurrent.TimeUnit

/**
 * HTTP client for the Qwen3-ASR service.
 *
 * Flow:
 *  1. POST /v1/asr  (multipart: audio file + language) → { "task_id": "..." }
 *  2. Poll GET /v1/tasks/{task_id} until status == completed | failed
 *  3. Return result.full_text on success
 */
class AsrClient(private val context: Context) {

    private val prefs: SharedPreferences by lazy {
        context.getSharedPreferences("asr_settings", Context.MODE_PRIVATE)
    }

    private val httpClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(30, TimeUnit.SECONDS)
            .readTimeout(60, TimeUnit.SECONDS)
            .writeTimeout(60, TimeUnit.SECONDS)
            .build()
    }

    // ---------- Settings helpers ----------

    fun getAsrUrl(): String =
        prefs.getString("asr_url", "http://192.168.1.100:8765") ?: "http://192.168.1.100:8765"

    fun getApiKey(): String =
        prefs.getString("api_key", "") ?: ""

    // ---------- Public API ----------

    /**
     * Upload WAV audio to the ASR service and poll until the transcription is ready.
     *
     * @param wavData   Raw WAV bytes (16 kHz, mono, 16-bit PCM with WAV header)
     * @param language  Optional language hint (e.g. "zh", "en")
     * @return Result.success(fullText) or Result.failure(exception)
     */
    suspend fun transcribe(
        wavData: ByteArray,
        language: String? = null
    ): Result<String> = withContext(Dispatchers.IO) {
        try {
            val taskId = submitTask(wavData, language)
                .getOrElse { return@withContext Result.failure(it) }

            pollTask(taskId)
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    // ---------- Internal ----------

    private fun submitTask(
        wavData: ByteArray,
        language: String?
    ): Result<String> {
        val baseUrl = getAsrUrl().trimEnd('/')
        val url = "$baseUrl/v1/asr"

        // Write WAV to temp file for multipart upload
        val tempFile = File(context.cacheDir, "upload_${System.currentTimeMillis()}.wav")
        tempFile.writeBytes(wavData)

        try {
            val fileBody = tempFile.asRequestBody("audio/wav".toMediaType())
            val multipartBuilder = MultipartBody.Builder()
                .setType(MultipartBody.FORM)
                .addFormDataPart("file", "audio.wav", fileBody)

            if (!language.isNullOrBlank()) {
                multipartBuilder.addFormDataPart("language", language)
            }

            val requestBuilder = Request.Builder()
                .url(url)
                .post(multipartBuilder.build())

            // Add auth header if API key is set
            val apiKey = getApiKey()
            if (apiKey.isNotBlank()) {
                requestBuilder.header("Authorization", "Bearer $apiKey")
            }

            val response = httpClient.newCall(requestBuilder.build()).execute()

            if (!response.isSuccessful) {
                val body = response.body?.string() ?: "Unknown error"
                return Result.failure(Exception("ASR submit failed (${response.code}): $body"))
            }

            val responseBody = response.body?.string()
                ?: return Result.failure(Exception("Empty response from ASR service"))

            val json = JSONObject(responseBody)
            val taskId = json.getString("task_id")
            return Result.success(taskId)
        } finally {
            tempFile.delete()
        }
    }

    private suspend fun pollTask(taskId: String): Result<String> {
        val baseUrl = getAsrUrl().trimEnd('/')
        val maxAttempts = 120 // 120 * 1s = 2 minutes max wait
        var attempt = 0

        while (attempt < maxAttempts) {
            attempt++
            delay(1000)

            val url = "$baseUrl/v1/tasks/$taskId"
            val requestBuilder = Request.Builder().url(url).get()

            val apiKey = getApiKey()
            if (apiKey.isNotBlank()) {
                requestBuilder.header("Authorization", "Bearer $apiKey")
            }

            val response = try {
                httpClient.newCall(requestBuilder.build()).execute()
            } catch (e: Exception) {
                continue // Retry on network errors
            }

            if (!response.isSuccessful) {
                continue
            }

            val responseBody = response.body?.string() ?: continue
            val json = JSONObject(responseBody)
            val status = json.getString("status")

            when (status) {
                "completed" -> {
                    val result = json.optJSONObject("result")
                    val text = result?.optString("full_text")
                        ?: return Result.failure(Exception("Completed but no text in result"))
                    return Result.success(text)
                }
                "failed" -> {
                    val errMsg = json.optString("error", "ASR task failed")
                    return Result.failure(Exception(errMsg))
                }
                // "processing" / "queued" → keep polling
            }
        }

        return Result.failure(Exception("ASR task timed out after ${maxAttempts}s"))
    }
}
