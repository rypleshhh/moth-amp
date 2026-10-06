import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import 'collection_tiles.dart';
import 'errors.dart';
import 'player_bar.dart';
import 'theme.dart';
import 'track_cover.dart';
import 'track_list.dart';

/// Страница исполнителя: популярное, все треки, альбомы.
class ArtistScreen extends StatefulWidget {
  const ArtistScreen({
    super.key,
    required this.id,
    required this.name,
    required this.player,
    required this.eq,
  });

  final String id;
  final String name;
  final PlayerController player;
  final EqController eq;

  @override
  State<ArtistScreen> createState() => _ArtistScreenState();
}

class _ArtistScreenState extends State<ArtistScreen> {
  late Future<ArtistPageDto> _page = artistPage(id: widget.id);

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<ArtistPageDto>(
      future: _page,
      builder: (context, snap) {
        final page = snap.data;
        return DefaultTabController(
          length: 3,
          child: Scaffold(
            appBar: AppBar(
              toolbarHeight: 72,
              title: Row(
                children: [
                  ClipOval(
                    child: TrackCover(url: page?.artist.coverUrl, size: 52),
                  ),
                  const SizedBox(width: 14),
                  Expanded(
                    child: Text(
                      widget.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontFamily: Moth.mono,
                        fontSize: 20,
                      ),
                    ),
                  ),
                ],
              ),
              bottom: const TabBar(
                isScrollable: true,
                tabAlignment: TabAlignment.start,
                tabs: [
                  Tab(text: 'популярное'),
                  Tab(text: 'все треки'),
                  Tab(text: 'альбомы'),
                ],
              ),
            ),
            body: snap.hasError
                ? Center(
                    child: TextButton(
                      onPressed: () => setState(() {
                        _page = artistPage(id: widget.id);
                      }),
                      child: Text(
                        '${errorText(snap.error!)}\nповторить',
                        textAlign: TextAlign.center,
                      ),
                    ),
                  )
                : page == null
                ? const Center(child: CircularProgressIndicator())
                : TabBarView(
                    children: [
                      TrackList(
                        load: () async => page.popularTracks,
                        player: widget.player,
                      ),
                      TrackList(
                        load: () => artistTracks(id: widget.id),
                        player: widget.player,
                      ),
                      _Albums(page: page, player: widget.player, eq: widget.eq),
                    ],
                  ),
            bottomNavigationBar: PlayerBar(
              player: widget.player,
              eq: widget.eq,
            ),
          ),
        );
      },
    );
  }
}

class _Albums extends StatelessWidget {
  const _Albums({required this.page, required this.player, required this.eq});

  final ArtistPageDto page;
  final PlayerController player;
  final EqController eq;

  @override
  Widget build(BuildContext context) {
    if (page.albums.isEmpty && page.alsoAlbums.isEmpty) {
      return const EmptyNote('альбомов нет');
    }
    return ListView(
      children: [
        for (final a in page.albums)
          AlbumTile(album: a, player: player, eq: eq),
        if (page.alsoAlbums.isNotEmpty) const _Header('сборники и участие'),
        for (final a in page.alsoAlbums)
          AlbumTile(album: a, player: player, eq: eq),
      ],
    );
  }
}

class _Header extends StatelessWidget {
  const _Header(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 18, 16, 6),
    child: Text(text, style: Moth.label),
  );
}
