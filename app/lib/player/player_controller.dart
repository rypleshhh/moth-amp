import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:media_kit/media_kit.dart';

import '../src/rust/api/cache.dart';
import '../src/rust/api/wave.dart';
import '../src/rust/api/yandex.dart';
import '../ui/errors.dart';

/// Очередь и воспроизведение. Ссылка на поток запрашивается у ядра прямо перед
/// проигрыванием: подписанные ссылки Яндекса живут недолго.
class PlayerController extends ChangeNotifier {
  PlayerController() {
    _subs = [
      _player.stream.playing.listen((v) {
        playing = v;
        if (v) {
          _switching = false;
        } else if (_player.state.completed) {
          // Трек доигран, сейчас включится следующий: это не пауза.
          _switching = true;
        }
        notifyListeners();
      }),
      _player.stream.completed.listen((done) {
        if (done) _advance(completed: true);
      }),
      _player.stream.position.listen((p) {
        _lastPosition = p;
        // Позиция ушла вперёд после сбоя — воспроизведение восстановилось.
        final at = _errorAt;
        if (at != null && p > at + const Duration(seconds: 1)) {
          _errorAt = null;
          _errorTimer?.cancel();
          if (error != null) {
            error = null;
            notifyListeners();
          }
        }
      }),
      _player.stream.duration.listen((d) => _lastDuration = d),
      _player.stream.error.listen(_onMpvError),
    ];
  }

  Timer? _errorTimer;
  Duration? _errorAt;

  /// Сообщения mpv о сбоях часто временные: соединение переподключается,
  /// а звук играет из буфера. Показываем ошибку, только если позиция
  /// не сдвинулась за 3 секунды.
  void _onMpvError(String e) {
    // Пока открывается новый трек, mpv сообщает об оборванной загрузке
    // предыдущего — это не ошибка текущего трека.
    if (loading) return;
    final at = _lastPosition;
    _errorAt = at;
    _errorTimer?.cancel();
    _errorTimer = Timer(const Duration(seconds: 3), () {
      if (!loading &&
          _errorAt == at &&
          _lastPosition <= at + const Duration(seconds: 1)) {
        error = e;
        _switching = false;
        notifyListeners();
      }
    });
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

  /// Переход к следующему треку: прошлый доигран, новый ещё грузится.
  bool _switching = false;

  /// Музыка должна играть: идёт воспроизведение или переход между треками.
  /// На Android, пока это так, сервис остаётся на переднем плане, а телефон
  /// не засыпает (иначе с выключенным экраном следующий трек не включится).
  bool get active => playing || _switching;

  /// Откуда играет текущий трек (кэш или сеть), кодек и битрейт.
  PlaySourceDto? stream;

  /// Играет «Моя волна»: очередь бесконечная, догружается партиями.
  bool waveActive = false;

  /// Обучаемый режим волны (отчёты уходят на сервер); иначе тихий.
  bool waveLearning = false;
  bool _waveLoadingMore = false;

  // Для отчётов волны: какой трек начал играть и сколько его слушали.
  String? _startedTrackId;
  Duration _lastPosition = Duration.zero;
  Duration _lastDuration = Duration.zero;

  TrackDto? get current =>
      _index >= 0 && _index < _queue.length ? _queue[_index] : null;
  List<TrackDto> get queue => _queue;
  int get index => _index;
  Stream<Duration> get position => _player.stream.position;
  Stream<Duration> get duration => _player.stream.duration;

  /// Длительность из данных Яндекса. Для потокового mp3 mpv по ходу загрузки
  /// уточняет свою оценку длительности, и ползунок «прыгает»; эта — точная.
  Duration? get knownDuration {
    final ms = current?.durationMs;
    return ms == null || ms <= 0 ? null : Duration(milliseconds: ms);
  }

  /// Обычная очередь (плейлист, лайки, поиск). Останавливает волну.
  Future<void> playQueue(List<TrackDto> tracks, int start) async {
    _finishCurrent(skipped: true);
    if (waveActive) _stopWaveSession();
    _queue = List.of(tracks);
    await _playAt(start);
  }

  /// Переход к треку текущей очереди (например, выбор в списке волны).
  Future<void> playIndex(int i) async {
    _finishCurrent(skipped: true);
    await _playAt(i);
  }

  Future<void> next() => _advance(completed: false);

  Future<void> previous() async {
    _finishCurrent(skipped: true);
    await _playAt(_findAvailable(_index - 1, -1));
  }

  /// Запустить «Мою волну». `learning = false` — тихий режим.
  Future<void> startWave({required bool learning}) async {
    _finishCurrent(skipped: true);
    final request = ++_request;
    loading = true;
    error = null;
    notifyListeners();
    try {
      final tracks = await waveStart(learning: learning);
      if (request != _request) return;
      waveActive = true;
      waveLearning = learning;
      _queue = List.of(tracks);
      _index = -1;
      await _playAt(_findAvailable(0, 1));
    } catch (e) {
      if (request != _request) return;
      error = errorText(e);
      loading = false;
      notifyListeners();
    }
  }

  Future<void> stopWave() async {
    _finishCurrent(skipped: true);
    _switching = false;
    _stopWaveSession();
    await _player.stop();
    _queue = const [];
    _index = -1;
    stream = null;
    notifyListeners();
  }

  void _stopWaveSession() {
    waveActive = false;
    waveStop().catchError((Object e) => debugPrint('waveStop: $e'));
  }

  Future<void> _advance({required bool completed}) async {
    _finishCurrent(skipped: !completed);
    var i = _findAvailable(_index + 1, 1);
    if (i < 0 && waveActive) {
      await _loadMoreWave();
      i = _findAvailable(_index + 1, 1);
    }
    if (i < 0) {
      // Очередь кончилась.
      _switching = false;
      notifyListeners();
      return;
    }
    await _playAt(i);
  }

  /// Отчёт об окончании текущего трека волны (дослушан или пропущен).
  void _finishCurrent({required bool skipped}) {
    final id = _startedTrackId;
    _startedTrackId = null;
    if (id == null || !waveActive) return;
    final played = skipped || _lastDuration == Duration.zero
        ? _lastPosition
        : _lastDuration;
    waveTrackEnded(
      trackId: id,
      playedSecs: played.inMilliseconds / 1000,
      skipped: skipped,
    ).catchError((Object e) => debugPrint('waveTrackEnded: $e'));
  }

  Future<void> _loadMoreWave() async {
    if (_waveLoadingMore) return;
    _waveLoadingMore = true;
    try {
      final more = await waveMore();
      if (!waveActive) return;
      _queue = [..._queue, ...more];
      notifyListeners();
    } catch (e) {
      debugPrint('waveMore: $e');
    } finally {
      _waveLoadingMore = false;
    }
  }

  Future<void> playOrPause() {
    _switching = false;
    return _player.playOrPause();
  }

  Future<void> play() => _player.play();

  Future<void> pause() {
    _switching = false;
    return _player.pause();
  }

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
    _switching = true;
    error = null;
    notifyListeners();
    try {
      final s = await playSource(track: _queue[i]);
      if (request != _request) return;
      stream = s;
      _lastPosition = Duration.zero;
      await _player.open(Media(s.url));
      final id = _queue[i].id;
      _startedTrackId = id;
      if (waveActive) {
        waveTrackStarted(trackId: id)
            .catchError((Object e) => debugPrint('waveTrackStarted: $e'));
        // Догружаем заранее, чтобы следующий трек был готов.
        if (i >= _queue.length - 2) _loadMoreWave();
      }
    } catch (e) {
      if (request != _request) return;
      error = errorText(e);
      _switching = false;
    } finally {
      if (request == _request) {
        loading = false;
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    _errorTimer?.cancel();
    _finishCurrent(skipped: true);
    for (final s in _subs) {
      s.cancel();
    }
    _player.dispose();
    super.dispose();
  }
}
