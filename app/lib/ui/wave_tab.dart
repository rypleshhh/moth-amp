import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import '../player/player_controller.dart';
import '../src/rust/api/wave.dart';
import '../src/rust/api/yandex.dart';
import '../audio/downloads.dart';
import 'theme.dart';
import 'track_cover.dart';
import 'track_list.dart';

/// «Моя волна»: режим, настройки (занятие, настроение, характер, язык),
/// запуск и бесконечная очередь.
class WaveTab extends StatefulWidget {
  const WaveTab({super.key, required this.player});

  final PlayerController player;

  @override
  State<WaveTab> createState() => _WaveTabState();
}

class _WaveTabState extends State<WaveTab> with AutomaticKeepAliveClientMixin {
  // Пока статус подписки не известен — тихий режим.
  bool _learning = false;
  bool _userChose = false;

  /// Варианты настроек от Яндекса; `null` — ещё не загружены или нет сети.
  WaveSettingsDto? _settings;

  /// Выбранное занятие (зерно) или `null`.
  String? _activity;

  /// Выбранный вариант в каждой группе: название группы → зерно.
  final Map<String, String> _chosen = {};

  @override
  bool get wantKeepAlive => true;

  @override
  void initState() {
    super.initState();
    // С Плюсом по умолчанию обычная волна, как в приложении Яндекса.
    account()
        .then((a) {
          if (mounted && !_userChose) setState(() => _learning = a.hasPlus);
        })
        .catchError((Object _) {});
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    try {
      final settings = await waveSettings();
      final saved = await _WaveChoice.load();
      if (!mounted) return;
      setState(() {
        _settings = settings;
        _activity = settings.activities
            .map((a) => a.seed)
            .where(saved.contains)
            .firstOrNull;
        for (final g in settings.groups) {
          final pick = g.options.where((o) => saved.contains(o.seed));
          if (pick.isNotEmpty) _chosen[g.name] = pick.first.seed;
        }
      });
    } catch (e) {
      debugPrint('waveSettings: $e');
    }
  }

  /// Зёрна для сессии: занятие (или обычная волна) и выбранные настройки,
  /// кроме вариантов «любое».
  List<String> get _seeds {
    final settings = _settings;
    if (settings == null) return const [];
    final defaults = {
      for (final g in settings.groups)
        for (final o in g.options)
          if (o.isDefault) o.seed,
    };
    final chosen = _chosen.values.where((s) => !defaults.contains(s));
    if (_activity == null && chosen.isEmpty) return const [];
    return [_activity ?? 'user:onyourwave', ...chosen];
  }

  /// Название волны по выбранным настройкам: «Тренируюсь · Бодрое».
  String? get _title {
    final settings = _settings;
    if (settings == null) return null;
    final names = [
      for (final a in settings.activities)
        if (a.seed == _activity) a.name,
      for (final g in settings.groups)
        for (final o in g.options)
          if (!o.isDefault && _chosen[g.name] == o.seed) o.name,
    ];
    return names.isEmpty ? null : names.join(' · ');
  }

  void _changed() {
    _WaveChoice.save(_seeds);
    // Как в приложении Яндекса: играющая волна сразу перестраивается.
    final player = widget.player;
    if (player.waveActive && !player.loading) {
      player.startWave(learning: _learning, seeds: _seeds);
    }
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final player = widget.player;
    return ListenableBuilder(
      listenable: player,
      builder: (context, _) {
        final active = player.waveActive;
        final queue = active ? player.queue : const <TrackDto>[];
        return ListView.builder(
          itemCount: queue.length + 1,
          itemBuilder: (context, i) {
            if (i == 0) return _header(context, active);
            return _QueueTile(player: player, index: i - 1);
          },
        );
      },
    );
  }

  Widget _header(BuildContext context, bool active) {
    final theme = Theme.of(context);
    final player = widget.player;
    final settings = _settings;
    final title = _title;
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'моя волна',
            style: TextStyle(
              fontFamily: Moth.mono,
              fontSize: 22,
              color: Moth.paper,
            ),
          ),
          const SizedBox(height: 2),
          Text(
            title == null
                ? 'бесконечный поток под ваше настроение'
                : title.toLowerCase(),
            style: Moth.label,
          ),
          const SizedBox(height: 12),
          SegmentedButton<bool>(
            segments: const [
              ButtonSegment(
                value: false,
                icon: Icon(Icons.visibility_off_outlined),
                label: Text('Тихая'),
              ),
              ButtonSegment(
                value: true,
                icon: Icon(Icons.auto_awesome_outlined),
                label: Text('Обычная'),
              ),
            ],
            selected: {_learning},
            onSelectionChanged: (v) => setState(() {
              _learning = v.first;
              _userChose = true;
            }),
          ),
          const SizedBox(height: 8),
          Text(
            _learning
                ? 'Как в приложении Яндекса: отправляются отчёты «начал / '
                      'дослушал / пропустил», и волна подстраивается под вас.'
                : 'Волна в режиме incognito, отчёты о прослушивании не '
                      'отправляются. Ваши рекомендации не меняются.',
            style: theme.textTheme.bodySmall,
          ),
          if (settings != null) ...[
            if (settings.activities.isNotEmpty)
              _ChipRow(
                label: 'занятие',
                children: [
                  for (final a in settings.activities)
                    ChoiceChip(
                      label: Text(a.name),
                      selected: _activity == a.seed,
                      onSelected: (on) {
                        setState(() => _activity = on ? a.seed : null);
                        _changed();
                      },
                    ),
                ],
              ),
            for (final g in settings.groups)
              _ChipRow(
                label: g.name.toLowerCase(),
                children: [
                  // «Любое» — первым.
                  for (final o in [
                    ...g.options.where((o) => o.isDefault),
                    ...g.options.where((o) => !o.isDefault),
                  ])
                    ChoiceChip(
                      label: Text(o.name),
                      selected:
                          (_chosen[g.name] ??
                              g.options
                                  .where((o) => o.isDefault)
                                  .firstOrNull
                                  ?.seed) ==
                          o.seed,
                      onSelected: (_) {
                        setState(() => _chosen[g.name] = o.seed);
                        _changed();
                      },
                    ),
                ],
              ),
          ],
          const SizedBox(height: 16),
          Wrap(
            spacing: 8,
            children: [
              FilledButton.icon(
                icon: Icon(active ? Icons.refresh : Icons.play_arrow),
                label: Text(active ? 'Перезапустить' : 'Слушать волну'),
                onPressed: player.loading
                    ? null
                    : () =>
                          player.startWave(learning: _learning, seeds: _seeds),
              ),
              if (active)
                OutlinedButton.icon(
                  icon: const Icon(Icons.stop),
                  label: const Text('Остановить'),
                  onPressed: player.stopWave,
                ),
            ],
          ),
          if (active && player.waveLearning != _learning)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                'Сейчас играет ${player.waveLearning ? 'обычная' : 'тихая'} '
                'волна. Новый режим применится после перезапуска.',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.tertiary,
                ),
              ),
            ),
          if (active) ...[const SizedBox(height: 16), const Divider(height: 1)],
        ],
      ),
    );
  }
}

/// Подпись и ряд чипов одной настройки.
class _ChipRow extends StatelessWidget {
  const _ChipRow({required this.label, required this.children});

  final String label;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(label, style: Moth.label),
          const SizedBox(height: 6),
          Wrap(spacing: 6, runSpacing: 6, children: children),
        ],
      ),
    );
  }
}

class _QueueTile extends StatelessWidget {
  const _QueueTile({required this.player, required this.index});

  final PlayerController player;
  final int index;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final t = player.queue[index];
    final ms = t.durationMs;
    return ListTile(
      leading: TrackCover(url: t.coverUrl),
      dense: true,
      enabled: t.available,
      selected: index == player.index,
      // Уже сыгранные треки приглушены.
      textColor: index < player.index ? theme.disabledColor : null,
      title: Text(t.title, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(t.artists, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (ms != null) Text(formatDuration(Duration(milliseconds: ms))),
          const SizedBox(width: 4),
          DownloadButton(track: t),
        ],
      ),
      onTap: () => player.playIndex(index),
    );
  }
}

/// Выбранные настройки волны между запусками: `wave.json` в папке приложения.
abstract final class _WaveChoice {
  static Future<File> _file() async =>
      File('${(await getApplicationSupportDirectory()).path}/wave.json');

  static Future<Set<String>> load() async {
    try {
      final f = await _file();
      if (!await f.exists()) return {};
      final json = jsonDecode(await f.readAsString()) as Map<String, dynamic>;
      return {...(json['seeds'] as List).cast<String>()};
    } catch (e) {
      debugPrint('wave.json: $e');
      return {};
    }
  }

  static Future<void> save(List<String> seeds) async {
    try {
      await (await _file()).writeAsString(jsonEncode({'seeds': seeds}));
    } catch (e) {
      debugPrint('wave.json: $e');
    }
  }
}
