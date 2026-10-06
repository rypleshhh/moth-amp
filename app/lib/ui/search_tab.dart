import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import 'collection_tiles.dart';
import 'errors.dart';
import 'theme.dart';
import 'track_list.dart';

/// Поиск по разделам: исполнители, треки, альбомы, плейлисты.
/// Раздел лучшего совпадения идёт первым.
class SearchTab extends StatefulWidget {
  const SearchTab({super.key, required this.player, required this.eq});

  final PlayerController player;
  final EqController eq;

  @override
  State<SearchTab> createState() => _SearchTabState();
}

class _SearchTabState extends State<SearchTab>
    with AutomaticKeepAliveClientMixin {
  Future<SearchDto>? _results;
  final _query = TextEditingController();

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  @override
  bool get wantKeepAlive => true;

  void _search(String text) {
    final q = text.trim();
    if (q.isEmpty) return;
    setState(() {
      _results = searchAll(query: q);
    });
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(12),
          child: TextField(
            controller: _query,
            decoration: InputDecoration(
              prefixIcon: const Icon(Icons.search),
              hintText: 'исполнитель, трек, альбом, плейлист',
              border: const OutlineInputBorder(),
              suffixIcon: IconButton(
                tooltip: 'Искать',
                icon: const Icon(Icons.arrow_forward),
                onPressed: () => _search(_query.text),
              ),
            ),
            textInputAction: TextInputAction.search,
            onSubmitted: _search,
          ),
        ),
        Expanded(
          child: _results == null
              ? const EmptyNote('что ищем?', icon: Icons.search)
              : FutureBuilder<SearchDto>(
                  future: _results,
                  builder: (context, snap) {
                    if (snap.hasError) {
                      return Center(child: Text(errorText(snap.error!)));
                    }
                    final r = snap.data;
                    if (r == null) {
                      return const Center(child: CircularProgressIndicator());
                    }
                    return _Results(
                      results: r,
                      player: widget.player,
                      eq: widget.eq,
                    );
                  },
                ),
        ),
      ],
    );
  }
}

class _Results extends StatelessWidget {
  const _Results({
    required this.results,
    required this.player,
    required this.eq,
  });

  final SearchDto results;
  final PlayerController player;
  final EqController eq;

  @override
  Widget build(BuildContext context) {
    final r = results;
    final sections = <String, List<Widget>>{
      'artist': [
        for (final a in r.artists)
          ArtistTile(artist: a, player: player, eq: eq),
      ],
      'track': [
        // Треки перерисовываются при смене текущего трека.
        if (r.tracks.isNotEmpty)
          ListenableBuilder(
            listenable: player,
            builder: (context, _) => Column(
              children: [
                for (var i = 0; i < r.tracks.length; i++)
                  TrackTile(
                    track: r.tracks[i],
                    current: r.tracks[i].id == player.current?.id,
                    onTap: () => player.playQueue(r.tracks, i),
                  ),
              ],
            ),
          ),
      ],
      'album': [
        for (final a in r.albums) AlbumTile(album: a, player: player, eq: eq),
      ],
      'playlist': [
        for (final p in r.playlists)
          PlaylistTile(playlist: p, player: player, eq: eq),
      ],
    };
    const titles = {
      'artist': 'исполнители',
      'track': 'треки',
      'album': 'альбомы',
      'playlist': 'плейлисты',
    };
    final order = ['artist', 'track', 'album', 'playlist'];
    final best = r.best;
    if (best != null && order.remove(best)) order.insert(0, best);

    final children = <Widget>[];
    for (final key in order) {
      final items = sections[key]!;
      if (items.isEmpty) continue;
      children
        ..add(
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 16, 16, 4),
            child: Text(titles[key]!, style: Moth.label),
          ),
        )
        ..addAll(items);
    }
    if (children.isEmpty) return const EmptyNote('ничего не нашлось');
    return ListView(children: children);
  }
}
