import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import 'equalizer_screen.dart';
import 'track_cover.dart';
import 'track_list.dart';

class PlayerBar extends StatelessWidget {
  const PlayerBar({super.key, required this.player, required this.eq});

  final PlayerController player;
  final EqController eq;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Material(
      color: theme.colorScheme.surfaceContainer,
      child: SafeArea(
        top: false,
        child: ListenableBuilder(
          listenable: player,
          builder: (context, _) {
            final track = player.current;
            final stream = player.stream;
            return Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                ProgressBar(player: player),
                Padding(
                  padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
                  // Левая и правая части одинаковой ширины, поэтому кнопки
                  // управления всегда точно по центру.
                  child: Row(
                    children: [
                      Expanded(
                        child: Row(
                          children: [
                            TrackCover(url: track?.coverUrl, size: 44),
                            const SizedBox(width: 12),
                            Expanded(
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  Text(
                                    track?.title ?? 'Ничего не играет',
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: theme.textTheme.titleSmall,
                                  ),
                                  Text(
                                    player.error ??
                                        [
                                          track?.artists ?? '',
                                          if (stream != null)
                                            '${stream.codec}'
                                                '${stream.bitrateKbps != null ? ' ${stream.bitrateKbps}' : ''}'
                                                '${stream.isPreview ? ' · превью' : ''}'
                                                '${stream.cached ? ' · из кэша' : ''}',
                                        ].where((s) => s.isNotEmpty).join(' · '),
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: theme.textTheme.bodySmall?.copyWith(
                                      color: player.error != null
                                          ? theme.colorScheme.error
                                          : null,
                                    ),
                                  ),
                                ],
                              ),
                            ),
                          ],
                        ),
                      ),
                      const SizedBox(width: 12),
                      IconButton(
                        icon: const Icon(Icons.skip_previous),
                        onPressed: track == null ? null : player.previous,
                      ),
                      player.loading
                          ? const Padding(
                              padding: EdgeInsets.all(12),
                              child: SizedBox.square(
                                dimension: 24,
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                ),
                              ),
                            )
                          : IconButton.filled(
                              icon: Icon(
                                player.playing ? Icons.pause : Icons.play_arrow,
                              ),
                              onPressed: track == null
                                  ? null
                                  : player.playOrPause,
                            ),
                      IconButton(
                        icon: const Icon(Icons.skip_next),
                        onPressed: track == null ? null : player.next,
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: Row(
                          mainAxisAlignment: MainAxisAlignment.end,
                          children: [
                            EqualizerButton(eq: eq),
                            Flexible(child: VolumeSlider(player: player)),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class ProgressBar extends StatefulWidget {
  const ProgressBar({super.key, required this.player});

  final PlayerController player;

  @override
  State<ProgressBar> createState() => _ProgressBarState();
}

class _ProgressBarState extends State<ProgressBar> {
  // Позиция, которую пользователь тянет ползунком (пока не отпустил).
  double? _dragMs;

  @override
  Widget build(BuildContext context) {
    return StreamBuilder<Duration>(
      stream: widget.player.duration,
      builder: (context, durSnap) {
        final total =
            widget.player.knownDuration ?? durSnap.data ?? Duration.zero;
        return StreamBuilder<Duration>(
          stream: widget.player.position,
          builder: (context, posSnap) {
            final pos = posSnap.data ?? Duration.zero;
            final max = total.inMilliseconds.toDouble();
            final value = (_dragMs ?? pos.inMilliseconds.toDouble()).clamp(
              0.0,
              max,
            );
            return Row(
              children: [
                const SizedBox(width: 12),
                Text(
                  formatDuration(Duration(milliseconds: value.round())),
                  style: Theme.of(context).textTheme.labelSmall,
                ),
                Expanded(
                  child: Slider(
                    value: max > 0 ? value : 0,
                    max: max > 0 ? max : 1,
                    onChanged: max > 0
                        ? (v) => setState(() => _dragMs = v)
                        : null,
                    onChangeEnd: (v) {
                      widget.player.seek(Duration(milliseconds: v.round()));
                      setState(() => _dragMs = null);
                    },
                  ),
                ),
                Text(
                  formatDuration(total),
                  style: Theme.of(context).textTheme.labelSmall,
                ),
                const SizedBox(width: 12),
              ],
            );
          },
        );
      },
    );
  }
}

class VolumeSlider extends StatelessWidget {
  const VolumeSlider({super.key, required this.player});

  final PlayerController player;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 120,
      // Перерисовывается вместе с PlayerBar (ListenableBuilder по player).
      child: Slider(
        value: player.userVolume,
        max: 100,
        onChanged: player.setVolume,
      ),
    );
  }
}

class EqualizerButton extends StatelessWidget {
  const EqualizerButton({super.key, required this.eq, this.onPressed});

  final EqController eq;

  /// По умолчанию открывает полный экран эквалайзера.
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: eq,
      builder: (context, _) {
        final s = eq.current;
        // Подсветка, только если эквалайзер реально меняет звук.
        final active =
            s.enabled &&
            (s.preampDb.abs() >= 0.01 ||
                s.bands.any((b) => b.gainDb.abs() >= 0.01));
        return IconButton(
          tooltip: 'Эквалайзер',
          isSelected: active,
          icon: const Icon(Icons.equalizer),
          onPressed:
              onPressed ??
              () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => EqualizerScreen(eq: eq),
                ),
              ),
        );
      },
    );
  }
}
