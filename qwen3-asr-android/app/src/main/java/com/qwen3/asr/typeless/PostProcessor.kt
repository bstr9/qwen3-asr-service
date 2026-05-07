package com.qwen3.asr.typeless

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Text post-processing pipeline for ASR output.
 *
 * Provides configurable text cleanup: filler removal, repetition removal,
 * auto-formatting, and optional LLM-based refinement.
 */
object PostProcessor {

    // ── Chinese fillers: exact character/phrase matches ────────────────

    private val CHINESE_FILLERS = listOf(
        "嗯", "呃", "啊", "那个", "就是说", "然后呢", "对对对",
    )

    // ── English fillers: regex with word boundaries ────────────────────

    private val ENGLISH_FILLER_PATTERNS = listOf(
        Regex("\\bum\\b"),
        Regex("\\buh\\b"),
        Regex("\\blike\\b"),
        Regex("\\byou know\\b"),
        Regex("\\bI mean\\b"),
    )

    private val MULTISPACE_RE = Regex("\\s{2,}")

    // ── Punctuation sets ───────────────────────────────────────────────

    private val CHINESE_END_PUNCTUATION = setOf('。', '！', '？', '…', '；')
    private val ENGLISH_END_PUNCTUATION = setOf('.', '!', '?', ';', ':')

    // ── LLM defaults ───────────────────────────────────────────────────

    private const val DEFAULT_SYSTEM_PROMPT =
        "You are a text post-processing assistant. Clean up the following speech-to-text output. " +
        "Remove filler words, fix grammar, add punctuation, and improve readability while " +
        "preserving the original meaning. Output only the cleaned text, nothing else."

    private val JSON_MEDIA_TYPE = "application/json; charset=utf-8".toMediaType()

    // ── OkHttp client for LLM calls ────────────────────────────────────

    private val llmHttpClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .build()
    }

    // =====================================================================
    // Public API
    // =====================================================================

    /**
     * Remove common Chinese and English filler words.
     *
     * Chinese fillers are replaced as exact substrings.
     * English fillers use regex with word boundaries to avoid
     * removing "like" from "likely" etc.
     */
    fun removeFillers(text: String): String {
        var result = text

        // Chinese fillers — simple string replacement
        for (filler in CHINESE_FILLERS) {
            result = result.replace(filler, "")
        }

        // English fillers — regex with word boundaries
        for (pattern in ENGLISH_FILLER_PATTERNS) {
            result = pattern.replace(result, "")
        }

        // Collapse multiple spaces into one, then trim
        result = MULTISPACE_RE.replace(result, " ")
        return result.trim()
    }

    /**
     * Remove consecutive duplicate phrases (3+ characters).
     *
     * Detects immediately repeated substrings and keeps only one instance.
     * Iterates until no more repetitions are found (handles nested repetitions).
     *
     * E.g. "今天天气很好今天天气很好" → "今天天气很好"
     */
    fun removeRepetitions(text: String): String {
        if (text.length < 6) return text

        var result = text
        var changed = true

        while (changed) {
            changed = false
            val chars = result.toList()
            if (chars.size < 6) break

            val maxPhraseChars = chars.size / 2

            for (phraseLen in maxPhraseChars downTo 3) {
                var i = 0
                while (i + phraseLen <= chars.size) {
                    val phrase = chars.subList(i, i + phraseLen)
                    val nextStart = i + phraseLen
                    val nextEnd = nextStart + phraseLen

                    if (nextEnd <= chars.size) {
                        val next = chars.subList(nextStart, nextEnd)
                        if (phrase == next) {
                            // Remove the duplicate: keep [0..i+phraseLen), skip [i+phraseLen..i+2*phraseLen)
                            val newChars = ArrayList<Char>(chars.size - phraseLen)
                            for (j in 0 until i + phraseLen) {
                                newChars.add(chars[j])
                            }
                            for (j in nextEnd until chars.size) {
                                newChars.add(chars[j])
                            }
                            result = String(newChars.toCharArray())
                            changed = true
                            break
                        }
                    }
                    i++
                }
                if (changed) break // Restart outer loop with new string
            }
        }

        return result
    }

    /**
     * Auto-format text: trim, capitalize first letter, fix spacing,
     * add trailing punctuation.
     *
     * - Trim whitespace
     * - Fix double spaces
     * - Capitalize first character (English)
     * - Ensure ending punctuation (add 。 for Chinese, . for English if missing)
     */
    fun autoFormat(text: String): String {
        var result = text.trim()
        if (result.isEmpty()) return result

        // Fix double spaces
        result = MULTISPACE_RE.replace(result, " ")

        // Capitalize first character for English text
        val firstChar = result.first()
        if (firstChar in 'a'..'z') {
            result = firstChar.uppercaseChar() + result.substring(1)
        }

        // Ensure ending punctuation
        val lastChar = result.last()
        val hasEndPunct = lastChar in CHINESE_END_PUNCTUATION ||
                          lastChar in ENGLISH_END_PUNCTUATION

        if (!hasEndPunct) {
            val isChinese = result.any { ch ->
                ch in '\u4E00'..'\u9FFF' || ch in '\u3400'..'\u4DBF'
            }
            result += if (isChinese) "。" else "."
        }

        return result
    }

    /**
     * Run the post-processing pipeline.
     *
     * Applies enabled steps in order:
     * removeFillers → removeRepetitions → autoFormat.
     */
    fun postprocess(
        text: String,
        removeFillers: Boolean,
        removeRepetitions: Boolean,
        autoFormat: Boolean,
    ): String {
        var result = text

        if (removeFillers) {
            result = this.removeFillers(result)
        }
        if (removeRepetitions) {
            result = this.removeRepetitions(result)
        }
        if (autoFormat) {
            result = this.autoFormat(result)
        }

        return result
    }

    /**
     * Optional LLM-based post-processing refinement.
     *
     * Sends the text to an OpenAI-compatible chat completion endpoint.
     * Falls back to returning the input text if the LLM call fails.
     *
     * @param text         The text to refine.
     * @param llmUrl       The chat completion endpoint URL.
     * @param llmModel     The model identifier to use.
     * @param llmApiKey    Optional API key for Bearer auth.
     * @param customPrompt Optional custom system prompt; defaults to a
     *                     standard post-processing prompt if null.
     * @param dictionaryHint Optional personal dictionary hint appended
     *                       to the system prompt.
     * @return The LLM-refined text, or the original text on failure.
     */
    suspend fun llmPostprocess(
        text: String,
        llmUrl: String,
        llmModel: String,
        llmApiKey: String?,
        customPrompt: String?,
        dictionaryHint: String?,
    ): String = withContext(Dispatchers.IO) {
        try {
            var systemPrompt = customPrompt ?: DEFAULT_SYSTEM_PROMPT

            // Append dictionary hint if provided and non-empty
            if (!dictionaryHint.isNullOrEmpty()) {
                systemPrompt += "\n\nThe user has a personal dictionary. Use these preferred spellings:\n$dictionaryHint"
            }

            // Build request body
            val messagesArray = JSONArray().apply {
                put(JSONObject().apply {
                    put("role", "system")
                    put("content", systemPrompt)
                })
                put(JSONObject().apply {
                    put("role", "user")
                    put("content", text)
                })
            }

            val requestBody = JSONObject().apply {
                put("model", llmModel)
                put("messages", messagesArray)
                put("temperature", 0.3)
            }

            val request = Request.Builder()
                .url(llmUrl)
                .post(requestBody.toString().toRequestBody(JSON_MEDIA_TYPE))
                .apply {
                    if (!llmApiKey.isNullOrBlank()) {
                        header("Authorization", "Bearer $llmApiKey")
                    }
                }
                .build()

            val response = llmHttpClient.newCall(request).execute()

            if (!response.isSuccessful) {
                return@withContext text
            }

            val responseBody = response.body?.string() ?: return@withContext text
            val responseJson = JSONObject(responseBody)

            val cleaned = responseJson
                .optJSONArray("choices")
                ?.optJSONObject(0)
                ?.optJSONObject("message")
                ?.optString("content")
                ?: return@withContext text

            cleaned
        } catch (_: Exception) {
            text
        }
    }
}
