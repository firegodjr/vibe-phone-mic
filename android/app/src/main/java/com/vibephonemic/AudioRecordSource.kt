package com.vibephonemic

import android.annotation.SuppressLint
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import com.vibephonemic.core.AudioSource

/** Microphone capture via [AudioRecord]: mono PCM16 at 48 kHz, falling back to 44.1 kHz. */
class AudioRecordSource(
    private val device: AudioDeviceInfo?,
    override val description: String,
) : AudioSource {
    override val watchdogEnabled: Boolean get() = true
    private var record: AudioRecord? = null
    override var sampleRate: Int = 48000
        private set

    @SuppressLint("MissingPermission")
    override fun start() {
        var lastError = "no supported sample rate"
        for (rate in intArrayOf(48000, 44100)) {
            val min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            if (min <= 0) continue
            val bufBytes = maxOf(min, rate / 100 * 2 * 8)
            val rec = try {
                AudioRecord(MediaRecorder.AudioSource.MIC, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, bufBytes)
            } catch (e: Exception) {
                lastError = e.message ?: e.toString(); continue
            }
            if (rec.state != AudioRecord.STATE_INITIALIZED) {
                rec.release(); lastError = "AudioRecord init failed at $rate Hz"; continue
            }
            if (device != null) rec.preferredDevice = device
            try {
                rec.startRecording()
            } catch (e: Exception) {
                rec.release(); lastError = e.message ?: e.toString(); continue
            }
            if (rec.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                rec.release(); lastError = "Microphone is busy"; continue
            }
            record = rec
            sampleRate = rate
            return
        }
        throw IllegalStateException(lastError)
    }

    override fun read(buf: ShortArray, count: Int): Int = record?.read(buf, 0, count) ?: -1

    override fun stop() {
        record?.let {
            try { it.stop() } catch (_: Exception) {}
            it.release()
        }
        record = null
    }
}
