import 'package:flutter/material.dart';

/// Обложки Яндекса отдаются в любом из стандартных размеров по суффиксу URL.
/// Для списков берём 100×100 вместо 400×400: в разы меньше трафика.
String thumbnailUrl(String url) =>
    url.replaceFirst(RegExp(r'/\d+x\d+$'), '/100x100');

/// Маленькая обложка трека для списков.
class TrackCover extends StatelessWidget {
  const TrackCover({super.key, required this.url, this.size = 40});

  final String? url;
  final double size;

  @override
  Widget build(BuildContext context) {
    final placeholder = Container(
      width: size,
      height: size,
      color: Theme.of(context).colorScheme.surfaceContainerHighest,
      child: Icon(Icons.music_note, size: size * 0.5),
    );
    final u = url;
    if (u == null) return placeholder;
    // Декодируем сразу в размер на экране, а не в исходный: меньше памяти.
    final px = (size * MediaQuery.devicePixelRatioOf(context)).round();
    return ClipRRect(
      borderRadius: BorderRadius.circular(4),
      child: Image.network(
        thumbnailUrl(u),
        width: size,
        height: size,
        fit: BoxFit.cover,
        cacheWidth: px,
        cacheHeight: px,
        gaplessPlayback: true,
        errorBuilder: (_, _, _) => placeholder,
        frameBuilder: (context, child, frame, sync) =>
            frame == null && !sync ? placeholder : child,
      ),
    );
  }
}
