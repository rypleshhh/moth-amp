import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../src/rust/api/eq.dart';
import 'deck.dart';
import 'equalizer_screen.dart';
import 'theme.dart';

/// Быстрый эквалайзер поверх обложки: вкл/выкл, пресет, предусилитель,
/// полосы. Всё остальное — в «Подробнее».
class QuickEq extends StatelessWidget {
  const QuickEq({super.key, required this.eq, required this.onClose});

  final EqController eq;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    const label = TextStyle(
      fontFamily: Moth.mono,
      fontSize: 10,
      color: DeckColors.amberDim,
    );
    return Bevel(
      inset: true,
      color: DeckColors.lcd.withValues(alpha: 0.94),
      padding: const EdgeInsets.fromLTRB(10, 6, 6, 8),
      child: ListenableBuilder(
        listenable: eq,
        builder: (context, _) {
          final s = eq.current;
          final range = s.mode == EqModeDto.parametric ? 24.0 : 12.0;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  const Text(
                    'ЭКВАЛАЙЗЕР',
                    style: TextStyle(
                      fontFamily: Moth.mono,
                      fontSize: 12,
                      color: Moth.amber,
                    ),
                  ),
                  const Spacer(),
                  Transform.scale(
                    scale: 0.8,
                    child: Switch(value: s.enabled, onChanged: eq.setEnabled),
                  ),
                  IconButton(
                    tooltip: 'Закрыть',
                    visualDensity: VisualDensity.compact,
                    icon: const Icon(Icons.close, size: 18),
                    onPressed: onClose,
                  ),
                ],
              ),
              _PresetPicker(eq: eq),
              Row(
                children: [
                  const SizedBox(width: 44, child: Text('PRE', style: label)),
                  Expanded(
                    child: Slider(
                      value: s.preampDb.clamp(-24.0, 12.0),
                      min: -24,
                      max: 12,
                      divisions: 72,
                      onChanged: s.enabled
                          ? (v) => eq.setPreamp(v, apply: false)
                          : null,
                      onChangeEnd: (_) => eq.commit(),
                    ),
                  ),
                  SizedBox(
                    width: 52,
                    child: Text(
                      '${s.preampDb.toStringAsFixed(1)} дБ',
                      style: label,
                    ),
                  ),
                  TextButton(
                    onPressed: s.enabled ? eq.autoPreamp : null,
                    child: const Text('Авто'),
                  ),
                ],
              ),
              Expanded(
                child: s.bands.isEmpty
                    ? const Center(
                        child: Text(
                          'Полос нет — откройте «Подробнее»',
                          style: label,
                        ),
                      )
                    : Row(
                        children: [
                          for (var i = 0; i < s.bands.length; i++)
                            Expanded(
                              child: _BandColumn(
                                band: s.bands[i],
                                range: range,
                                enabled: s.enabled,
                                labelStyle: label,
                                onChanged: (v) => eq.setBand(
                                  i,
                                  s.bands[i].copyWith(gainDb: v),
                                  apply: false,
                                ),
                                onChangeEnd: eq.commit,
                              ),
                            ),
                        ],
                      ),
              ),
              Align(
                alignment: Alignment.centerRight,
                child: TextButton.icon(
                  icon: const Icon(Icons.tune, size: 16),
                  label: const Text('Подробнее'),
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => EqualizerScreen(eq: eq),
                    ),
                  ),
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _BandColumn extends StatelessWidget {
  const _BandColumn({
    required this.band,
    required this.range,
    required this.enabled,
    required this.labelStyle,
    required this.onChanged,
    required this.onChangeEnd,
  });

  final EqBandDto band;
  final double range;
  final bool enabled;
  final TextStyle labelStyle;
  final ValueChanged<double> onChanged;
  final VoidCallback onChangeEnd;

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Expanded(
          child: RotatedBox(
            quarterTurns: 3,
            child: SliderTheme(
              // Узкие ползунки, чтобы влезли и 18 полос.
              data: SliderTheme.of(context).copyWith(
                trackHeight: 2,
                thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 5),
                overlayShape: const RoundSliderOverlayShape(overlayRadius: 10),
              ),
              child: Slider(
                value: band.gainDb.clamp(-range, range),
                min: -range,
                max: range,
                onChanged: enabled ? onChanged : null,
                onChangeEnd: (_) => onChangeEnd(),
              ),
            ),
          ),
        ),
        FittedBox(child: Text(formatFreq(band.freqHz), style: labelStyle)),
      ],
    );
  }
}

class _PresetPicker extends StatelessWidget {
  const _PresetPicker({required this.eq});

  final EqController eq;

  @override
  Widget build(BuildContext context) {
    final presets = [...eq.builtinPresets, ...eq.userPresets];
    return MenuAnchor(
      builder: (context, menu, _) => Align(
        alignment: Alignment.centerLeft,
        child: TextButton.icon(
          icon: const Icon(Icons.arrow_drop_down, size: 18),
          label: Text(eq.presetName ?? 'Пресет'),
          onPressed: () => menu.isOpen ? menu.close() : menu.open(),
        ),
      ),
      menuChildren: [
        for (final p in presets)
          MenuItemButton(
            onPressed: () => eq.applyPreset(p),
            child: Text(p.name),
          ),
      ],
    );
  }
}
