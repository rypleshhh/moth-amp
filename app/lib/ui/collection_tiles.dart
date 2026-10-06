import 'package:flutter/material.dart';

import '../audio/downloads.dart';
import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import 'artist_screen.dart';
import 'collection_screen.dart';
import 'theme.dart';
import 'track_cover.dart';

/// Число треков и кнопка «скачать всё» справа в строке плейлиста/альбома.
Widget _countAndDownload({
  required int? count,
  required String folder,
  required Future<List<TrackDto>> Function() load,
  required String? coverUrl,
}) {
  return Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      if (count != null) Text('$count', style: Moth.label),
      PlaylistDownloadButton(name: folder, load: load, coverUrl: coverUrl),
    ],
  );
}

/// Строка плейлиста: обложка, название, число треков, скачать.
class PlaylistTile extends StatelessWidget {
  const PlaylistTile({
    super.key,
    required this.playlist,
    required this.player,
    required this.eq,
  });

  final PlaylistDto playlist;
  final PlayerController player;
  final EqController eq;

  Future<List<TrackDto>> _load() =>
      playlistTracks(id: playlist.id, ownerUid: playlist.ownerUid);

  @override
  Widget build(BuildContext context) {
    final p = playlist;
    return ListTile(
      leading: TrackCover(url: p.coverUrl),
      title: Text(p.title, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: _countAndDownload(
        count: p.trackCount,
        folder: p.title,
        load: _load,
        coverUrl: p.coverUrl,
      ),
      onTap: () => Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => CollectionScreen(
            title: p.title,
            subtitle: p.trackCount == null ? null : 'треков: ${p.trackCount}',
            coverUrl: p.coverUrl,
            folderName: p.title,
            load: _load,
            player: player,
            eq: eq,
          ),
        ),
      ),
    );
  }
}

/// Строка альбома: обложка, название, исполнитель · год, скачать.
class AlbumTile extends StatelessWidget {
  const AlbumTile({
    super.key,
    required this.album,
    required this.player,
    required this.eq,
  });

  final AlbumDto album;
  final PlayerController player;
  final EqController eq;

  Future<List<TrackDto>> _load() => albumTracks(id: album.id);

  @override
  Widget build(BuildContext context) {
    final a = album;
    final sub = [a.artists, if (a.year != null) '${a.year}'].join(' · ');
    return ListTile(
      leading: TrackCover(url: a.coverUrl),
      title: Text(a.title, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(sub, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: _countAndDownload(
        count: a.trackCount,
        folder: a.folderName,
        load: _load,
        coverUrl: a.coverUrl,
      ),
      onTap: () => Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => CollectionScreen(
            title: a.title,
            subtitle: sub,
            coverUrl: a.coverUrl,
            folderName: a.folderName,
            load: _load,
            player: player,
            eq: eq,
          ),
        ),
      ),
    );
  }
}

/// Строка исполнителя: круглый аватар, имя; открывает страницу исполнителя.
class ArtistTile extends StatelessWidget {
  const ArtistTile({
    super.key,
    required this.artist,
    required this.player,
    required this.eq,
  });

  final ArtistDto artist;
  final PlayerController player;
  final EqController eq;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: ClipOval(child: TrackCover(url: artist.coverUrl)),
      title: Text(artist.name, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: const Icon(Icons.chevron_right),
      onTap: () => openArtist(context, artist.id, artist.name, player, eq),
    );
  }
}

void openArtist(
  BuildContext context,
  String id,
  String name,
  PlayerController player,
  EqController eq,
) {
  Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ArtistScreen(id: id, name: name, player: player, eq: eq),
    ),
  );
}
