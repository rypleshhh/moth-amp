import 'package:flutter/material.dart';

import '../audio/downloads.dart';
import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import 'player_bar.dart';
import 'theme.dart';
import 'track_cover.dart';
import 'track_list.dart';

/// Экран плейлиста или альбома: обложка и название в шапке, треки,
/// кнопка «скачать всё» в папку `<загрузки>/<folderName>/`.
class CollectionScreen extends StatelessWidget {
  const CollectionScreen({
    super.key,
    required this.title,
    this.subtitle,
    this.coverUrl,
    required this.folderName,
    required this.load,
    required this.player,
    required this.eq,
  });

  final String title;
  final String? subtitle;
  final String? coverUrl;
  final String folderName;
  final Future<List<TrackDto>> Function() load;
  final PlayerController player;
  final EqController eq;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        toolbarHeight: 64,
        title: Row(
          children: [
            TrackCover(url: coverUrl, size: 44),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(title, maxLines: 1, overflow: TextOverflow.ellipsis),
                  if (subtitle != null)
                    Text(subtitle!, style: Moth.label, maxLines: 1),
                ],
              ),
            ),
          ],
        ),
        actions: [
          PlaylistDownloadButton(
            name: folderName,
            load: load,
            coverUrl: coverUrl,
          ),
          const SizedBox(width: 8),
        ],
      ),
      body: TrackList(load: load, player: player),
      bottomNavigationBar: PlayerBar(player: player, eq: eq),
    );
  }
}
