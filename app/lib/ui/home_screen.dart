import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import 'errors.dart';
import 'player_bar.dart';
import 'track_list.dart';

class HomeScreen extends StatelessWidget {
  const HomeScreen({
    super.key,
    required this.player,
    required this.eq,
    required this.onLogout,
  });

  final PlayerController player;
  final EqController eq;
  final VoidCallback onLogout;

  Future<void> _logout() async {
    await logout();
    onLogout();
  }

  @override
  Widget build(BuildContext context) {
    return DefaultTabController(
      length: 3,
      child: Scaffold(
        appBar: AppBar(
          title: FutureBuilder<AccountDto>(
            future: account(),
            builder: (context, snap) {
              final acc = snap.data;
              if (acc == null) return const Text('moth-amp');
              return Text('${acc.name}${acc.hasPlus ? ' · Плюс' : ''}');
            },
          ),
          actions: [
            IconButton(
              tooltip: 'Выйти',
              icon: const Icon(Icons.logout),
              onPressed: _logout,
            ),
          ],
          bottom: const TabBar(
            tabs: [
              Tab(text: 'Мне нравится'),
              Tab(text: 'Плейлисты'),
              Tab(text: 'Поиск'),
            ],
          ),
        ),
        body: TabBarView(
          children: [
            TrackList(load: likedTracks, player: player),
            _PlaylistsTab(player: player, eq: eq),
            _SearchTab(player: player),
          ],
        ),
        bottomNavigationBar: PlayerBar(player: player, eq: eq),
      ),
    );
  }
}

class _PlaylistsTab extends StatefulWidget {
  const _PlaylistsTab({required this.player, required this.eq});

  final PlayerController player;
  final EqController eq;

  @override
  State<_PlaylistsTab> createState() => _PlaylistsTabState();
}

class _PlaylistsTabState extends State<_PlaylistsTab>
    with AutomaticKeepAliveClientMixin {
  final _future = playlists();

  @override
  bool get wantKeepAlive => true;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return FutureBuilder<List<PlaylistDto>>(
      future: _future,
      builder: (context, snap) {
        if (snap.hasError) return Center(child: Text(errorText(snap.error!)));
        final list = snap.data;
        if (list == null) {
          return const Center(child: CircularProgressIndicator());
        }
        return ListView.builder(
          itemCount: list.length,
          itemBuilder: (context, i) {
            final p = list[i];
            return ListTile(
              leading: const Icon(Icons.queue_music),
              title: Text(p.title),
              trailing: p.trackCount == null ? null : Text('${p.trackCount}'),
              onTap: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => Scaffold(
                    appBar: AppBar(title: Text(p.title)),
                    body: TrackList(
                      load: () => playlistTracks(id: p.id),
                      player: widget.player,
                    ),
                    bottomNavigationBar: PlayerBar(
                      player: widget.player,
                      eq: widget.eq,
                    ),
                  ),
                ),
              ),
            );
          },
        );
      },
    );
  }
}

class _SearchTab extends StatefulWidget {
  const _SearchTab({required this.player});

  final PlayerController player;

  @override
  State<_SearchTab> createState() => _SearchTabState();
}

class _SearchTabState extends State<_SearchTab>
    with AutomaticKeepAliveClientMixin {
  String? _query;

  @override
  bool get wantKeepAlive => true;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final query = _query;
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.all(12),
          child: TextField(
            decoration: const InputDecoration(
              prefixIcon: Icon(Icons.search),
              hintText: 'Исполнитель, трек, альбом',
              border: OutlineInputBorder(),
            ),
            textInputAction: TextInputAction.search,
            onSubmitted: (v) {
              if (v.trim().isNotEmpty) setState(() => _query = v.trim());
            },
          ),
        ),
        Expanded(
          child: query == null
              ? const SizedBox.shrink()
              : TrackList(
                  key: ValueKey(query),
                  load: () => search(query: query),
                  player: widget.player,
                ),
        ),
      ],
    );
  }
}
