import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import 'collection_tiles.dart';
import 'errors.dart';
import 'theme.dart';

/// Лайкнутые альбомы; открытый альбом работает как плейлист.
class AlbumsTab extends StatefulWidget {
  const AlbumsTab({super.key, required this.player, required this.eq});

  final PlayerController player;
  final EqController eq;

  @override
  State<AlbumsTab> createState() => _AlbumsTabState();
}

class _AlbumsTabState extends State<AlbumsTab>
    with AutomaticKeepAliveClientMixin {
  late Future<List<AlbumDto>> _future = likedAlbums();

  @override
  bool get wantKeepAlive => true;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return FutureBuilder<List<AlbumDto>>(
      future: _future,
      builder: (context, snap) {
        if (snap.hasError) {
          return Center(
            child: TextButton(
              onPressed: () => setState(() {
                _future = likedAlbums();
              }),
              child: Text(
                '${errorText(snap.error!)}\nповторить',
                textAlign: TextAlign.center,
              ),
            ),
          );
        }
        final list = snap.data;
        if (list == null) {
          return const Center(child: CircularProgressIndicator());
        }
        if (list.isEmpty) return const EmptyNote('лайкнутых альбомов пока нет');
        return ListView.builder(
          itemCount: list.length,
          itemBuilder: (context, i) =>
              AlbumTile(album: list[i], player: widget.player, eq: widget.eq),
        );
      },
    );
  }
}
