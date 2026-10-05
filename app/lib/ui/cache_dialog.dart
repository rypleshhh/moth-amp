import 'package:flutter/material.dart';

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
                  const SizedBox(height: 16),
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
                  Text(
                    'Прослушанные треки сохраняются как mp3/flac и в следующий раз '
                    'играют без сети. При переполнении удаляются давно не '
                    'игравшие. Кэш работает, пока подписка подтверждена (до 30 '
                    'дней без сети), и удаляется при выходе из аккаунта.',
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
