import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:media_kit/media_kit.dart';

import '../src/rust/api/yandex.dart';
import '../ui/errors.dart';

/// Очередь и воспроизведение. Ссылка на поток запрашивается у ядра прямо перед
/// проигрыванием: подписанные ссылки Яндекса живут недолго.
class PlayerController extends ChangeNotifier {
  PlayerController() {
    _subs = [
      _player.stream.playing.listen((v) {
        playing = v;
        notifyListeners();
      }),
      _player.stream.completed.listen((done) {
        if (done) next();
      }),
      _player.stream.error.listen((e) {
        error = e;
        notifyListeners();
      }),
    ];
  }

  final Player _player = Player();
  late final List<StreamSubscription<Object?>> _subs;

  List<TrackDto> _queue = const [];
  int _index = -1;
  // Номер последнего запроса: ответ на устаревший запрос игнорируется.
  int _request = 0;

  bool playing = false;
  bool loading = false;
  String? error;
  StreamDto? stream;

  TrackDto? get current =>
      _index >= 0 && _index < _queue.length ? _queue[_index] : null;
  Stream<Duration> get position => _player.stream.position;
  Stream<Duration> get duration => _player.stream.duration;

  Future<void> playQueue(List<TrackDto> tracks, int start) async {
    _queue = List.of(tracks);
    await _playAt(start);
  }

  Future<void> next() => _playAt(_findAvailable(_index + 1, 1));

  Future<void> previous() => _playAt(_findAvailable(_index - 1, -1));

  Future<void> playOrPause() => _player.playOrPause();

  Future<void> play() => _player.play();

  Future<void> pause() => _player.pause();

  Future<void> seek(Duration position) => _player.seek(position);

  double _userVolume = 100;
  double _preampDb = 0;

  /// Громкость, выставленная пользователем (0–100), без учёта предусилителя.
  double get userVolume => _userVolume;

  /// 0–100.
  Future<void> setVolume(double value) {
    _userVolume = value.clamp(0, 100).toDouble();
    notifyListeners();
    return _applyVolume();
  }

  /// Предусилитель эквалайзера. В сборке FFmpeg из media_kit нет фильтра
  /// `volume`, поэтому усиление применяется громкостью плеера.
  Future<void> setPreampDb(double db) {
    _preampDb = db;
    return _applyVolume();
  }

  // Громкость mpv кубическая: амплитуда = (volume/100)^3, поэтому
  // усиление в дБ переводится в множитель громкости 10^(дБ/60).
  // Выше 100 не поднимаем: положительный предусилитель упирается в потолок.
  Future<void> _applyVolume() {
    final v = _userVolume * math.pow(10, _preampDb / 60);
    return _player.setVolume(v.clamp(0, 100).toDouble());
  }

  /// Цепочка звуковых фильтров mpv (`af`); пустая строка — без обработки.
  /// Свойство сохраняется между треками.
  Future<void> setAudioFilter(String af) async {
    final platform = _player.platform;
    if (platform is NativePlayer) {
      await platform.setProperty('af', af);
    }
  }

  int _findAvailable(int from, int step) {
    for (var i = from; i >= 0 && i < _queue.length; i += step) {
      if (_queue[i].available) return i;
    }
    return -1;
  }

  Future<void> _playAt(int i) async {
    if (i < 0 || i >= _queue.length) return;
    final request = ++_request;
    _index = i;
    loading = true;
    error = null;
    notifyListeners();
    try {
      final s = await streamUrl(trackId: _queue[i].id, lowQuality: false);
      if (request != _request) return;
      stream = s;
      await _player.open(Media(s.url));
    } catch (e) {
      if (request != _request) return;
      error = errorText(e);
    } finally {
      if (request == _request) {
        loading = false;
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    for (final s in _subs) {
      s.cancel();
    }
    _player.dispose();
    super.dispose();
  }
}
