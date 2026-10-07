import 'package:flutter/material.dart';

import '../src/rust/api/cache.dart';
import '../src/rust/api/yandex.dart';
import '../ui/errors.dart';

/// Какие треки в кэше и какие сейчас скачиваются (для иконок в списках).
class DownloadController extends ChangeNotifier {
  final Set<String> _cached = {};
  final Set<String> _downloading = {};

  /// Растёт при каждом изменении состава кэша (для перезагрузки «Скачанного»).
  int version = 0;

  /// Идущие загрузки плейлистов: название → (готово, всего).
  final Map<String, (int, int)> playlistProgress = {};

  bool isCached(String id) => _cached.contains(id);
  bool isDownloading(String id) => _downloading.contains(id);

  /// Перечитать список из кэша (после воспроизведения трек мог сохраниться сам).
  Future<void> refresh() async {
    try {
      final ids = await cachedIds();
      if (ids.length != _cached.length || !ids.every(_cached.contains)) {
        _cached
          ..clear()
          ..addAll(ids);
        version++;
        notifyListeners();
      }
    } catch (e) {
      debugPrint('cachedIds: $e');
    }
  }

  /// Дописать метаданные старым записям кэша и скопировать кэш в S3
  /// (один раз за запуск, в фоне).
  Future<void> backfillMeta() async {
    try {
      if (await cacheBackfillMeta() > 0) {
        version++;
        notifyListeners();
      }
    } catch (e) {
      debugPrint('cacheBackfillMeta: $e');
    }
    try {
      final copied = await cacheMirrorToS3();
      if (copied > 0) debugPrint('В S3 скопировано треков: $copied');
    } catch (e) {
      debugPrint('cacheMirrorToS3: $e');
    }
  }

  /// Идёт синхронизация с S3.
  bool syncing = false;

  /// «Синхронизировать»: сверить списки с бакетом и загрузить в S3
  /// скачанное на этом устройстве. Ошибка — исключением.
  Future<S3SyncDto> syncS3() async {
    syncing = true;
    notifyListeners();
    try {
      return await s3Sync();
    } finally {
      syncing = false;
      version++;
      notifyListeners();
    }
  }

  /// Скачать плейлист в `<папка загрузок>/<название>/`: треки по одному,
  /// недоступные пропускаются, сбой одного не останавливает остальные.
  /// Возвращает (скачано, не удалось).
  Future<(int, int)> downloadPlaylist(
    String name,
    List<TrackDto> tracks,
  ) async {
    final list = tracks.where((t) => t.available).toList();
    var ok = 0;
    var failed = 0;
    playlistProgress[name] = (0, list.length);
    notifyListeners();
    for (var i = 0; i < list.length; i++) {
      final t = list[i];
      final error = await download(t);
      if (error == null && isCached(t.id)) {
        try {
          await cachePlaceInFolder(trackId: t.id, folder: name);
          ok++;
        } catch (e) {
          debugPrint('cachePlaceInFolder: $e');
          failed++;
        }
      } else {
        failed++;
      }
      playlistProgress[name] = (i + 1, list.length);
      notifyListeners();
    }
    playlistProgress.remove(name);
    version++;
    notifyListeners();
    return (ok, failed);
  }

  /// Скачать трек. Возвращает текст ошибки или `null`.
  Future<String?> download(TrackDto track) async {
    if (isCached(track.id) || isDownloading(track.id)) return null;
    _downloading.add(track.id);
    notifyListeners();
    try {
      await cacheDownload(track: track);
      _cached.add(track.id);
      version++;
      return null;
    } catch (e) {
      return errorText(e);
    } finally {
      _downloading.remove(track.id);
      notifyListeners();
    }
  }
}

/// Делает [DownloadController] доступным во всех экранах, включая открытые
/// поверх (плейлист и т.п.).
class DownloadsScope extends InheritedNotifier<DownloadController> {
  const DownloadsScope({
    super.key,
    required DownloadController controller,
    required super.child,
  }) : super(notifier: controller);

  static DownloadController of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<DownloadsScope>()!.notifier!;
}

/// Иконка у трека: скачать / качается / скачан.
class DownloadButton extends StatelessWidget {
  const DownloadButton({super.key, required this.track});

  final TrackDto track;

  @override
  Widget build(BuildContext context) {
    final downloads = DownloadsScope.of(context);
    final color = Theme.of(context).colorScheme.primary;
    if (downloads.isCached(track.id)) {
      return Tooltip(
        message: 'Сохранён в кэше',
        child: SizedBox.square(
          dimension: 40,
          child: Icon(Icons.download_done, size: 20, color: color),
        ),
      );
    }
    if (downloads.isDownloading(track.id)) {
      return const SizedBox.square(
        dimension: 40,
        child: Center(
          child: SizedBox.square(
            dimension: 18,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      );
    }
    return IconButton(
      tooltip: 'Скачать',
      iconSize: 20,
      icon: const Icon(Icons.download_outlined),
      onPressed: track.available
          ? () async {
              final messenger = ScaffoldMessenger.of(context);
              final error = await downloads.download(track);
              if (error != null) {
                messenger.showSnackBar(SnackBar(content: Text(error)));
              }
            }
          : null,
    );
  }
}

/// Кнопка «скачать плейлист» с прогрессом «12/40».
class PlaylistDownloadButton extends StatelessWidget {
  const PlaylistDownloadButton({
    super.key,
    required this.name,
    required this.load,
    this.coverUrl,
  });

  final String name;
  final Future<List<TrackDto>> Function() load;

  /// Обложка — сохраняется в папку как `folder.jpg`.
  final String? coverUrl;

  @override
  Widget build(BuildContext context) {
    final downloads = DownloadsScope.of(context);
    final progress = downloads.playlistProgress[name];
    if (progress != null) {
      final (done, total) = progress;
      return Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SizedBox.square(
              dimension: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            const SizedBox(width: 8),
            Text('$done/$total'),
          ],
        ),
      );
    }
    return IconButton(
      tooltip: 'Скачать всё в папку «$name»',
      icon: const Icon(Icons.download_for_offline_outlined),
      onPressed: () async {
        final messenger = ScaffoldMessenger.of(context);
        try {
          final tracks = await load();
          final (ok, failed) = await downloads.downloadPlaylist(name, tracks);
          final cover = coverUrl;
          if (ok > 0 && cover != null) {
            // Обложка папки не обязательна: ошибку не показываем.
            await cachePlaceFolderCover(
              folder: name,
              coverUrl: cover,
            ).catchError((Object e) => debugPrint('folder.jpg: $e'));
          }
          messenger.showSnackBar(
            SnackBar(
              content: Text(
                '«$name»: скачано $ok'
                '${failed > 0 ? ', не удалось $failed' : ''}',
              ),
            ),
          );
        } catch (e) {
          messenger.showSnackBar(SnackBar(content: Text(errorText(e))));
        }
      },
    );
  }
}
