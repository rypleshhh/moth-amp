import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';

import '../player/player_controller.dart';
import '../src/rust/api/s3.dart';
import 'errors.dart';
import 's3_dialog.dart';
import 'track_list.dart';

/// «Моя музыка»: собственные mp3/flac в S3.
class MyMusicTab extends StatefulWidget {
  const MyMusicTab({super.key, required this.player});

  final PlayerController player;

  @override
  State<MyMusicTab> createState() => _MyMusicTabState();
}

class _MyMusicTabState extends State<MyMusicTab>
    with AutomaticKeepAliveClientMixin {
  late Future<S3StatusDto> _status = s3Status();
  int _version = 0;
  bool _uploading = false;

  @override
  bool get wantKeepAlive => true;

  void _reload() => setState(() {
    _status = s3Status();
    _version++;
  });

  Future<void> _settings() async {
    if (await showS3Dialog(context)) _reload();
  }

  Future<void> _upload() async {
    final messenger = ScaffoldMessenger.of(context);
    final files = await openFiles(
      acceptedTypeGroups: const [
        XTypeGroup(label: 'Аудио mp3/flac', extensions: ['mp3', 'flac']),
      ],
      confirmButtonText: 'Загрузить',
    );
    if (files.isEmpty) return;
    setState(() => _uploading = true);
    try {
      final r = await s3Upload(paths: files.map((f) => f.path).toList());
      final failed = r.failed.isEmpty
          ? ''
          : '\nНе загружены:\n${r.failed.join('\n')}';
      messenger.showSnackBar(
        SnackBar(
          content: Text('Загружено: ${r.uploaded}$failed'),
          duration: Duration(seconds: r.failed.isEmpty ? 4 : 10),
        ),
      );
    } catch (e) {
      messenger.showSnackBar(SnackBar(content: Text(errorText(e))));
    } finally {
      if (mounted) {
        setState(() {
          _uploading = false;
          _version++;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return FutureBuilder<S3StatusDto>(
      future: _status,
      builder: (context, snap) {
        if (snap.hasError) return Center(child: Text(errorText(snap.error!)));
        final status = snap.data;
        if (status == null) {
          return const Center(child: CircularProgressIndicator());
        }
        if (!status.connected) return _NotConnected(onConnect: _settings);
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
              child: Row(
                children: [
                  FilledButton.icon(
                    icon: _uploading
                        ? const SizedBox.square(
                            dimension: 16,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(Icons.upload_file),
                    label: Text(
                      _uploading ? 'Загрузка…' : 'Загрузить mp3/flac',
                    ),
                    onPressed: _uploading ? null : _upload,
                  ),
                  const SizedBox(width: 8),
                  IconButton(
                    tooltip: 'Обновить',
                    icon: const Icon(Icons.refresh),
                    onPressed: () => setState(() => _version++),
                  ),
                  const Spacer(),
                  Text(
                    '${status.bucket} · ${Uri.tryParse(status.endpoint)?.host ?? status.endpoint}',
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                  IconButton(
                    tooltip: 'Настройки S3',
                    icon: const Icon(Icons.settings_outlined),
                    onPressed: _settings,
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
            Expanded(
              child: TrackList(
                key: ValueKey(_version),
                load: s3Tracks,
                player: widget.player,
              ),
            ),
          ],
        );
      },
    );
  }
}

class _NotConnected extends StatelessWidget {
  const _NotConnected({required this.onConnect});

  final VoidCallback onConnect;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 460),
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.cloud_outlined, size: 48),
              const SizedBox(height: 12),
              Text('Своя музыка в облаке', style: theme.textTheme.titleLarge),
              const SizedBox(height: 8),
              Text(
                'Подключите S3-совместимое хранилище, загружайте свои mp3 и flac '
                'и слушайте их на всех устройствах. Теги и обложки берутся из '
                'файлов, прослушанное сохраняется в кэш для офлайна.',
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium,
              ),
              const SizedBox(height: 16),
              FilledButton.icon(
                icon: const Icon(Icons.link),
                label: const Text('Подключить S3'),
                onPressed: onConnect,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
