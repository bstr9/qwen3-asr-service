package com.qwen3.asr.typeless

import android.app.Application

class App : Application() {

    val asrClient: AsrClient by lazy {
        AsrClient(this)
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    companion object {
        @Volatile
        private lateinit var instance: App

        fun getInstance(): App = instance
    }
}
