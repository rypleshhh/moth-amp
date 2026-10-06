package io.github.rypleshhh.moth_amp

import android.content.Context
import android.net.wifi.WifiManager
import android.os.PowerManager
import com.ryanheise.audioservice.AudioServiceActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

// AudioServiceActivity — обычная FlutterActivity, связанная с сервисом
// фонового воспроизведения audio_service.
class MainActivity : AudioServiceActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "moth_amp/power")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "hold" -> {
                        PlaybackLocks.hold(applicationContext, call.arguments == true)
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
    }
}

/// Пока музыка играет, телефон не должен засыпать: mpv не держит блокировок
/// сам, и с выключенным экраном процессор и Wi-Fi засыпают между треками —
/// следующий не загружается. Блокировки общие на процесс: движок Flutter
/// переживает пересоздание Activity.
private object PlaybackLocks {
    private var wake: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null

    @Synchronized
    fun hold(context: Context, on: Boolean) {
        if (wake == null) {
            val power = context.getSystemService(Context.POWER_SERVICE) as PowerManager
            wake = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "moth-amp:playback")
                .apply { setReferenceCounted(false) }
            val wifiManager = context.getSystemService(Context.WIFI_SERVICE) as WifiManager
            @Suppress("DEPRECATION")
            wifi = wifiManager.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "moth-amp:playback")
                .apply { setReferenceCounted(false) }
        }
        if (on) {
            wake?.acquire()
            wifi?.acquire()
        } else {
            wake?.takeIf { it.isHeld }?.release()
            wifi?.takeIf { it.isHeld }?.release()
        }
    }
}
