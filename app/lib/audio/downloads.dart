import 'package:flutter/material.dart';

import '../src/rust/api/cache.dart';
import '../src/rust/api/yandex.dart';
import '../ui/errors.dart';

/// Какие треки в кэше и какие сейчас скачиваются (для иконок в списках).
class DownloadController extends ChangeNotifier {
  final Set<String> _cached = {};
  final Set<String> _downloading = {};

  bool isCached(String id) => _cached.contains(id);
  bool isDownloading(String id) => _downloading.contains(id);

  /// Перечитать список из кэша (после воспроизведения трек мог сохраниться сам).
  Future<void> refresh() async {
    try {
      final ids = await cachedIds();
      _cached
        ..clear()
        ..addAll(ids);
      notifyListeners();
    } catch (e) {
      debugPrint('cachedIds: $e');
    }
  }

  /// Скачать трек. Возвращает текст ошибки или `null`.
  Future<String?> download(TrackDto track) async {
    if (isCached(track.id) || isDownloading(track.id)) return null;
    _downloading.add(track.id);
    notifyListeners();
    try {
      await cacheDownload(track: track);
      _cached.add(track.id);
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
