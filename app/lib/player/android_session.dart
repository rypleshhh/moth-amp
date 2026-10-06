import 'dart:async';
import 'dart:io';

import 'package:audio_service/audio_service.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'player_controller.dart';

/// Android: фоновое воспроизведение (foreground-сервис), уведомление с
/// кнопками, экран блокировки, гарнитура и Bluetooth-кнопки.
class AndroidSession {
  AndroidSession._(this._handler);

  final _MothAudioHandler _handler;

  /// `null` на платформах, кроме Android.
  static Future<AndroidSession?> attach(PlayerController player) async {
    if (!Platform.isAndroid) return null;
    final handler = await AudioService.init(
      builder: () => _MothAudioHandler(player),
      config: const AudioServiceConfig(
        androidNotificationChannelId: 'io.github.rypleshhh.moth_amp.playback',
        androidNotificationChannelName: 'Воспроизведение',
        androidNotificationIcon: 'mipmap/ic_launcher',
        // Уведомление можно смахнуть на паузе — сервис тогда не держит процесс.
        androidStopForegroundOnPause: true,
      ),
    );
    return AndroidSession._(handler);
  }

  Future<void> dispose() => _handler.dispose();
}

class _MothAudioHandler extends BaseAudioHandler with SeekHandler {
  _MothAudioHandler(this._player) {
    _player.addListener(_sync);
    _subs = [
      _player.position.listen((p) {
        _position = p;
        _pushState();
      }),
    ];
    _sync();
  }

  final PlayerController _player;
  late final List<StreamSubscription<Object?>> _subs;

  /// Блокировка от сна и Wi-Fi-блокировка (MainActivity.kt): без них телефон
  /// с выключенным экраном засыпает между треками.
  static const _power = MethodChannel('moth_amp/power');
  bool _awake = false;
  Duration _position = Duration.zero;
  String? _shownId;
  DateTime _lastPush = DateTime.fromMillisecondsSinceEpoch(0);

  void _sync() {
    final t = _player.current;
    if (t != null && t.id != _shownId) {
      _shownId = t.id;
      final ms = t.durationMs;
      mediaItem.add(
        MediaItem(
          id: t.id,
          title: t.title,
          artist: t.artists,
          album: t.album,
          duration: ms == null ? null : Duration(milliseconds: ms),
          artUri: t.coverUrl == null ? null : Uri.tryParse(t.coverUrl!),
        ),
      );
    }
    _pushState(force: true);
    _holdAwake(_player.active);
  }

  void _holdAwake(bool on) {
    if (on == _awake) return;
    _awake = on;
    _power
        .invokeMethod<void>('hold', on)
        .catchError((Object e) => debugPrint('power: $e'));
  }

  /// Состояние для уведомления; позиция — не чаще раза в секунду.
  void _pushState({bool force = false}) {
    final now = DateTime.now();
    if (!force && now.difference(_lastPush) < const Duration(seconds: 1)) {
      return;
    }
    _lastPush = now;
    final hasTrack = _player.current != null;
    // Между треками mpv сообщает «не играет»; для системы это всё ещё
    // воспроизведение, иначе сервис уйдёт с переднего плана.
    final playing = _player.active;
    playbackState.add(
      PlaybackState(
        controls: [
          MediaControl.skipToPrevious,
          playing ? MediaControl.pause : MediaControl.play,
          MediaControl.skipToNext,
        ],
        systemActions: const {MediaAction.seek},
        androidCompactActionIndices: const [0, 1, 2],
        processingState: !hasTrack
            ? AudioProcessingState.idle
            : _player.loading
            ? AudioProcessingState.loading
            : AudioProcessingState.ready,
        playing: playing,
        updatePosition: _position,
      ),
    );
  }

  @override
  Future<void> play() => _player.play();

  @override
  Future<void> pause() => _player.pause();

  @override
  Future<void> skipToNext() => _player.next();

  @override
  Future<void> skipToPrevious() => _player.previous();

  @override
  Future<void> seek(Duration position) => _player.seek(position);

  @override
  Future<void> stop() async {
    await _player.pause();
    await super.stop();
  }

  Future<void> dispose() async {
    _player.removeListener(_sync);
    _holdAwake(false);
    for (final s in _subs) {
      await s.cancel();
    }
  }
}
