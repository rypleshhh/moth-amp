import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:url_launcher/url_launcher.dart';

import '../src/rust/api/cache.dart';
import 'errors.dart';

/// Настройки кэша: занятое место, лимит, очистка.
Future<void> showCacheDialog(BuildContext context) =>
    showDialog<void>(context: context, builder: (_) => const _CacheDialog());

class _CacheDialog extends StatefulWidget {
  const _CacheDialog();

  @override
  State<_CacheDialog> createState() => _CacheDialogState();
}

class _CacheDialogState extends State<_CacheDialog> {
  static const _limits = [512, 1024, 2048, 5120, 10240];

  CacheStatsDto? _stats;
  String? _error;

  @override
  void initState() {
    super.initState();
    _reload();
  }

  Future<void> _reload() async {
    try {
      final s = await cacheStats();
      if (mounted) setState(() => _stats = s);
    } catch (e) {
      if (mounted) setState(() => _error = errorText(e));
    }
  }

  bool _moving = false;

  /// Выбор папки. На Android системный выбор отдаёт не путь, а разрешение на
  /// папку, поэтому там два варианта: память приложения или его папка в
  /// общей памяти (видна в файловом менеджере).
  Future<String?> _pickFolder() async {
    if (!Platform.isAndroid) {
      return getDirectoryPath(confirmButtonText: 'Выбрать');
    }
    final external = await getExternalStorageDirectory();
    if (!mounted) return null;
    return showDialog<String>(
      context: context,
      builder: (context) => SimpleDialog(
        title: const Text('Где хранить треки'),
        children: [
          SimpleDialogOption(
            onPressed: () => Navigator.pop(context, ''),
            child: const Text('Память приложения (по умолчанию)'),
          ),
          if (external != null)
            SimpleDialogOption(
              onPressed: () => Navigator.pop(context, '${external.path}/music'),
              child: Text('Общая память: ${external.path}/music'),
            ),
        ],
      ),
    );
  }

  Future<void> _changeFolder({bool reset = false}) async {
    final messenger = ScaffoldMessenger.of(context);
    final picked = reset ? '' : await _pickFolder();
    if (picked == null) return;
    setState(() => _moving = true);
    try {
      final moved = await cacheSetFolder(
        folder: picked.isEmpty ? null : picked,
      );
      messenger.showSnackBar(
        SnackBar(content: Text('Папка изменена, перенесено файлов: $moved')),
      );
    } catch (e) {
      messenger.showSnackBar(SnackBar(content: Text(errorText(e))));
    } finally {
      if (mounted) setState(() => _moving = false);
      await _reload();
    }
  }

  String _size(double mb) =>
      mb >= 1024 ? '${(mb / 1024).toStringAsFixed(1)} ГБ' : '${mb.round()} МБ';

  @override
  Widget build(BuildContext context) {
    final s = _stats;
    final theme = Theme.of(context);
    return AlertDialog(
      title: const Text('Кэш'),
      content: SizedBox(
        width: 380,
        child: _error != null
            ? Text(_error!)
            : s == null
            ? const LinearProgressIndicator()
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Занято ${_size(s.usedMb)} из ${_size(s.limitMb.toDouble())} · '
                    'треков: ${s.tracks}',
                  ),
                  const SizedBox(height: 8),
                  LinearProgressIndicator(
                    value: s.limitMb == 0
                        ? 0
                        : (s.usedMb / s.limitMb).clamp(0, 1),
                  ),
                  const SizedBox(height: 8),
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('Сохранять все прослушанные треки'),
                    subtitle: const Text(
                      'Выключено — в кэш попадают только треки, скачанные '
                      'кнопкой загрузки.',
                    ),
                    value: s.autoCache,
                    onChanged: (v) async {
                      await cacheSetAuto(enabled: v);
                      await _reload();
                    },
                  ),
                  const SizedBox(height: 8),
                  Text('Лимит', style: theme.textTheme.labelLarge),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 8,
                    children: [
                      for (final mb in _limits)
                        ChoiceChip(
                          label: Text(_size(mb.toDouble())),
                          selected: s.limitMb == mb,
                          onSelected: (_) async {
                            await cacheSetLimit(limitMb: mb);
                            await _reload();
                          },
                        ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  Text('Папка', style: theme.textTheme.labelLarge),
                  const SizedBox(height: 4),
                  SelectableText(s.folder, style: theme.textTheme.bodySmall),
                  Wrap(
                    spacing: 4,
                    children: [
                      TextButton.icon(
                        icon: _moving
                            ? const SizedBox.square(
                                dimension: 16,
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                ),
                              )
                            : const Icon(Icons.drive_folder_upload_outlined),
                        label: Text(_moving ? 'Переношу…' : 'Выбрать папку…'),
                        onPressed: _moving ? null : _changeFolder,
                      ),
                      if (s.customFolder)
                        TextButton(
                          onPressed: _moving
                              ? null
                              : () => _changeFolder(reset: true),
                          child: const Text('По умолчанию'),
                        ),
                      if (!Platform.isAndroid)
                        TextButton.icon(
                          icon: const Icon(Icons.folder_open_outlined),
                          label: const Text('Открыть'),
                          onPressed: () => launchUrl(Uri.directory(s.folder)),
                        ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Треки хранятся как обычные mp3/flac с тегами (название, исполнители, '
                    'альбом, год, обложка) и именами вида «Исполнитель — Название (id)». '
                    'При смене папки скачанные файлы переезжают в новую. Скачанное '
                    'играет без сети; при переполнении удаляются давно не игравшие. '
                    'Треки Яндекса играют, пока подписка подтверждена (до 30 дней без '
                    'сети); после выхода из аккаунта файлы остаются.',
                    style: theme.textTheme.bodySmall,
                  ),
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: s == null || s.tracks == 0
              ? null
              : () async {
                  await cacheClear();
                  await _reload();
                },
          child: const Text('Очистить'),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Готово'),
        ),
      ],
    );
  }
}
