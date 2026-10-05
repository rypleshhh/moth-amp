import 'dart:async';
import 'dart:io';

import 'package:smtc_windows/smtc_windows.dart';

import '../src/rust/api/yandex.dart';
import 'player_controller.dart';

/// Медиаклавиши и системная плитка Windows (System Media Transport Controls).
///
/// Плитка включается при первом треке, чтобы не висела пустой.
class MediaControls {
  MediaControls._(this._player, this._smtc) {
    _subs = [
      _smtc.buttonPressStream.listen(_onButton),
      _player.duration.listen((d) {
        _duration = d;
        _pushTimeline(force: true);
      }),
      _player.position.listen((p) {
        _position = p;
        _pushTimeline();
      }),
    ];
    _player.addListener(_sync);
  }

  /// `null` на платформах без SMTC.
  static Future<MediaControls?> attach(PlayerController player) async {
    if (!Platform.isWindows) return null;
    await SMTCWindows.initialize();
    final smtc = SMTCWindows(
      enabled: false,
      config: const SMTCConfig(
        playEnabled: true,
        pauseEnabled: true,
        nextEnabled: true,
        prevEnabled: true,
        stopEnabled: false,
        fastForwardEnabled: false,
        rewindEnabled: false,
      ),
    );
    return MediaControls._(player, smtc);
  }

  final PlayerController _player;
  final SMTCWindows _smtc;
  late final List<StreamSubscription<Object?>> _subs;

  TrackDto? _shownTrack;
  bool? _shownPlaying;
  Duration _duration = Duration.zero;
  Duration _position = Duration.zero;
  DateTime _lastTimelinePush = DateTime.fromMillisecondsSinceEpoch(0);

  void _onButton(PressedButton button) {
    switch (button) {
      case PressedButton.play:
        _player.play();
      case PressedButton.pause:
        _player.pause();
      case PressedButton.next:
        _player.next();
      case PressedButton.previous:
        _player.previous();
      default:
        break;
    }
  }

  void _sync() {
    final track = _player.current;
    if (track == null) return;

    if (!identical(track, _shownTrack)) {
      _shownTrack = track;
      if (!_smtc.enabled) _smtc.enableSmtc();
      _smtc.updateMetadata(
        MusicMetadata(
          title: track.title,
          artist: track.artists,
          album: track.album,
          thumbnail: track.coverUrl,
        ),
      );
    }

    if (_player.playing != _shownPlaying) {
      _shownPlaying = _player.playing;
      _smtc.setPlaybackStatus(
        _player.playing ? PlaybackStatus.playing : PlaybackStatus.paused,
      );
    }
  }

  /// Позиция для плитки обновляется не чаще раза в секунду.
  void _pushTimeline({bool force = false}) {
    if (!_smtc.enabled) return;
    final now = DateTime.now();
    if (!force &&
        now.difference(_lastTimelinePush) < const Duration(seconds: 1)) {
      return;
    }
    _lastTimelinePush = now;
    _smtc.updateTimeline(
      PlaybackTimeline(
        startTimeMs: 0,
        endTimeMs: _duration.inMilliseconds,
        positionMs: _position.inMilliseconds,
        minSeekTimeMs: 0,
        maxSeekTimeMs: _duration.inMilliseconds,
      ),
    );
  }

  Future<void> dispose() async {
    _player.removeListener(_sync);
    for (final s in _subs) {
      await s.cancel();
    }
    await _smtc.dispose();
  }
}
