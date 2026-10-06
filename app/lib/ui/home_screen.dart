import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import '../audio/downloads.dart';
import '../src/rust/api/cache.dart';
import 'cache_dialog.dart';
import 'deck.dart';
import 'theme.dart';
import 'my_music_tab.dart';
import 'errors.dart';
import 'player_bar.dart';
import 'track_list.dart';
import 'wave_tab.dart';

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

  /// С какой ширины окна показывать деку слева вместо нижней панели.
  static const _wideBreakpoint = 900.0;

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= _wideBreakpoint;
    final tabs = TabBarView(
      children: [
        WaveTab(player: player),
        TrackList(load: likedTracks, player: player),
        _PlaylistsTab(player: player, eq: eq),
        _SearchTab(player: player),
        // Из кэша, без сети. Перезагружается, когда меняется состав кэша.
        TrackList(
          key: ValueKey(DownloadsScope.of(context).version),
          load: cachedTracks,
          player: player,
        ),
        MyMusicTab(player: player),
      ],
    );
    return DefaultTabController(
      length: 6,
      child: Scaffold(
        appBar: AppBar(
          title: Row(
            children: [
              Image.asset('assets/logo.png', width: 22, height: 22),
              const SizedBox(width: 10),
              const Text(
                'moth-amp',
                style: TextStyle(fontFamily: Moth.mono, fontSize: 18),
              ),
            ],
          ),
          actions: [
            FutureBuilder<AccountDto>(
              future: account(),
              builder: (context, snap) {
                final acc = snap.data;
                if (acc == null) return const SizedBox.shrink();
                return Padding(
                  padding: const EdgeInsets.only(right: 8),
                  child: Text(
                    '${acc.name.toLowerCase()}${acc.hasPlus ? ' · плюс' : ''}',
                    style: Moth.label,
                  ),
                );
              },
            ),
            Builder(
              builder: (context) => IconButton(
                tooltip: 'Кэш',
                icon: const Icon(Icons.storage_outlined),
                onPressed: () => showCacheDialog(context),
              ),
            ),
            IconButton(
              tooltip: 'Выйти',
              icon: const Icon(Icons.logout),
              onPressed: _logout,
            ),
          ],
          bottom: const TabBar(
            isScrollable: true,
            tabAlignment: TabAlignment.start,
            tabs: [
              Tab(text: 'моя волна'),
              Tab(text: 'мне нравится'),
              Tab(text: 'плейлисты'),
              Tab(text: 'поиск'),
              Tab(text: 'скачанное'),
              Tab(text: 'моя музыка'),
            ],
          ),
        ),
        body: wide
            ? Row(
                children: [
                  SizedBox(
                    width: 360,
                    child: ClassicDeck(player: player, eq: eq),
                  ),
                  const VerticalDivider(width: 1),
                  Expanded(child: tabs),
                ],
              )
            : tabs,
        // На широком окне управление в деке слева.
        bottomNavigationBar: wide ? null : PlayerBar(player: player, eq: eq),
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
