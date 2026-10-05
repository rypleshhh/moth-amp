import 'package:flutter/material.dart';

import '../player/player_controller.dart';
import '../src/rust/api/yandex.dart';
import '../audio/downloads.dart';
import 'track_cover.dart';
import 'track_list.dart';

/// «Моя волна»: выбор режима, запуск и бесконечная очередь.
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
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final theme = Theme.of(context);
    final player = widget.player;
    return ListenableBuilder(
      listenable: player,
      builder: (context, _) {
        final active = player.waveActive;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('Моя волна', style: theme.textTheme.headlineSmall),
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
                  const SizedBox(height: 16),
                  Wrap(
                    spacing: 8,
                    children: [
                      FilledButton.icon(
                        icon: Icon(active ? Icons.refresh : Icons.play_arrow),
                        label: Text(active ? 'Перезапустить' : 'Слушать волну'),
                        onPressed: player.loading
                            ? null
                            : () => player.startWave(learning: _learning),
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
                ],
              ),
            ),
            if (active) const Divider(height: 1),
            if (active)
              Expanded(
                child: ListView.builder(
                  itemCount: player.queue.length,
                  itemBuilder: (context, i) {
                    final t = player.queue[i];
                    final ms = t.durationMs;
                    return ListTile(
                      leading: TrackCover(url: t.coverUrl),
                      dense: true,
                      enabled: t.available,
                      selected: i == player.index,
                      // Уже сыгранные треки приглушены.
                      textColor: i < player.index ? theme.disabledColor : null,
                      title: Text(
                        t.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                      subtitle: Text(
                        t.artists,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                      trailing: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          if (ms != null)
                            Text(formatDuration(Duration(milliseconds: ms))),
                          const SizedBox(width: 4),
                          DownloadButton(track: t),
                        ],
                      ),
                      onTap: () => player.playIndex(i),
                    );
                  },
                ),
              ),
          ],
        );
      },
    );
  }
}
