import 'package:flutter/material.dart';

import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import '../audio/downloads.dart';
import 'errors.dart';
import 'theme.dart';
import 'track_cover.dart';

String formatDuration(Duration d) {
  final m = d.inMinutes;
  final s = d.inSeconds % 60;
  return '$m:${s.toString().padLeft(2, '0')}';
}

/// Строка трека: обложка, название, исполнители, длительность, загрузка.
class TrackTile extends StatelessWidget {
  const TrackTile({
    super.key,
    required this.track,
    required this.current,
    required this.onTap,
  });

  final TrackDto track;

  /// Этот трек сейчас играет.
  final bool current;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = track;
    final ms = t.durationMs;
    return ListTile(
      leading: TrackCover(url: t.coverUrl),
      enabled: t.available,
      selected: current,
      dense: true,
      title: Text(
        current ? '▸ ${t.title}' : t.title,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
      subtitle: Text(t.artists, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (ms != null)
            Text(formatDuration(Duration(milliseconds: ms)), style: Moth.label),
          const SizedBox(width: 4),
          DownloadButton(track: t),
        ],
      ),
      onTap: onTap,
    );
  }
}

class TrackList extends StatefulWidget {
  const TrackList({super.key, required this.load, required this.player});

  final Future<List<TrackDto>> Function() load;
  final PlayerController player;

  @override
  State<TrackList> createState() => _TrackListState();
}

class _TrackListState extends State<TrackList>
    with AutomaticKeepAliveClientMixin {
  late Future<List<TrackDto>> _future = widget.load();

  @override
  bool get wantKeepAlive => true;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return FutureBuilder<List<TrackDto>>(
      future: _future,
      builder: (context, snap) {
        if (snap.hasError) {
          return Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(errorText(snap.error!), textAlign: TextAlign.center),
                TextButton(
                  onPressed: () => setState(() {
                    _future = widget.load();
                  }),
                  child: const Text('Повторить'),
                ),
              ],
            ),
          );
        }
        final tracks = snap.data;
        if (tracks == null) {
          return const Center(child: CircularProgressIndicator());
        }
        if (tracks.isEmpty) return const EmptyNote('тут пока пусто');

        return ListenableBuilder(
          listenable: widget.player,
          builder: (context, _) {
            final currentId = widget.player.current?.id;
            return ListView.builder(
              itemCount: tracks.length,
              itemBuilder: (context, i) => TrackTile(
                track: tracks[i],
                current: tracks[i].id == currentId,
                onTap: () => widget.player.playQueue(tracks, i),
              ),
            );
          },
        );
      },
    );
  }
}
