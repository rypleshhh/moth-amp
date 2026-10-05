import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../src/rust/api/eq.dart';
import 'errors.dart';

String _formatFreq(double hz) {
  if (hz >= 1000) {
    final k = hz / 1000;
    return '${k == k.roundToDouble() ? k.round() : k.toStringAsFixed(1)}k';
  }
  return hz.round().toString();
}

String _formatDb(double db) => '${db > 0 ? '+' : ''}${db.toStringAsFixed(1)}';

class EqualizerScreen extends StatelessWidget {
  const EqualizerScreen({super.key, required this.eq});

  final EqController eq;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: eq,
      builder: (context, _) {
        final s = eq.current;
        return Scaffold(
          appBar: AppBar(
            title: const Text('Эквалайзер'),
            actions: [
              Switch(value: s.enabled, onChanged: eq.setEnabled),
              const SizedBox(width: 8),
            ],
          ),
          body: AbsorbPointer(
            absorbing: !s.enabled,
            child: Opacity(
              opacity: s.enabled ? 1 : 0.45,
              child: ListView(
                padding: const EdgeInsets.all(16),
                children: [
                  SegmentedButton<EqModeDto>(
                    segments: const [
                      ButtonSegment(
                        value: EqModeDto.graphic10,
                        label: Text('10 полос'),
                      ),
                      ButtonSegment(
                        value: EqModeDto.graphic18,
                        label: Text('18 полос'),
                      ),
                      ButtonSegment(
                        value: EqModeDto.parametric,
                        label: Text('Параметрический'),
                      ),
                    ],
                    selected: {s.mode},
                    onSelectionChanged: (v) => eq.setMode(v.first),
                  ),
                  const SizedBox(height: 16),
                  _PresetBar(eq: eq),
                  const SizedBox(height: 8),
                  _PreampRow(eq: eq),
                  const Divider(height: 32),
                  if (eq.isGraphic)
                    _GraphicBands(eq: eq)
                  else
                    _ParametricBands(eq: eq),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

class _PresetBar extends StatelessWidget {
  const _PresetBar({required this.eq});

  final EqController eq;

  Future<void> _saveDialog(BuildContext context) async {
    final controller = TextEditingController(text: eq.presetName);
    final name = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Сохранить пресет'),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: const InputDecoration(labelText: 'Название'),
          onSubmitted: (v) => Navigator.pop(context, v),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Отмена'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: const Text('Сохранить'),
          ),
        ],
      ),
    );
    if (name != null && name.trim().isNotEmpty) eq.saveAsPreset(name.trim());
  }

  Future<void> _importDialog(BuildContext context) async {
    final controller = TextEditingController();
    String? error;
    await showDialog<void>(
      context: context,
      builder: (context) => StatefulBuilder(
        builder: (context, setState) => AlertDialog(
          title: const Text('Импорт AutoEq'),
          content: SizedBox(
            width: 480,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Вставьте содержимое файла «… ParametricEQ.txt» из AutoEq '
                  'или пресет Equalizer APO.',
                ),
                const SizedBox(height: 12),
                TextField(
                  controller: controller,
                  maxLines: 10,
                  minLines: 6,
                  style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
                  decoration: InputDecoration(
                    border: const OutlineInputBorder(),
                    hintText: 'Preamp: -6.2 dB\nFilter 1: ON PK Fc 105 Hz Gain 5.5 dB Q 0.70',
                    errorText: error,
                    errorMaxLines: 3,
                  ),
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Отмена'),
            ),
            FilledButton(
              onPressed: () {
                try {
                  eq.importAutoEq(controller.text);
                  Navigator.pop(context);
                } catch (e) {
                  setState(() => error = errorText(e));
                }
              },
              child: const Text('Импорт'),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Wrap(
      spacing: 8,
      runSpacing: 8,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        MenuAnchor(
          builder: (context, menu, _) => OutlinedButton.icon(
            icon: const Icon(Icons.tune),
            label: Text(eq.presetName ?? 'Пресеты'),
            onPressed: () => menu.isOpen ? menu.close() : menu.open(),
          ),
          menuChildren: [
            for (final p in eq.builtinPresets)
              MenuItemButton(
                onPressed: () => eq.applyPreset(p),
                child: Text(p.name),
              ),
            if (eq.userPresets.isNotEmpty) const Divider(),
            for (final p in eq.userPresets)
              MenuItemButton(
                onPressed: () => eq.applyPreset(p),
                trailingIcon: IconButton(
                  tooltip: 'Удалить пресет',
                  icon: const Icon(Icons.delete_outline, size: 18),
                  onPressed: () => eq.deletePreset(p.name),
                ),
                child: Text(
                  p.settings.mode == eq.current.mode
                      ? p.name
                      : '${p.name} (${_modeName(p.settings.mode)})',
                ),
              ),
          ],
        ),
        TextButton.icon(
          icon: const Icon(Icons.save_outlined),
          label: const Text('Сохранить'),
          onPressed: () => _saveDialog(context),
        ),
        TextButton.icon(
          icon: const Icon(Icons.file_download_outlined),
          label: const Text('Импорт AutoEq'),
          onPressed: () => _importDialog(context),
        ),
        TextButton.icon(
          icon: const Icon(Icons.restart_alt),
          label: const Text('Сбросить'),
          onPressed: eq.reset,
        ),
        if (eq.current.bands.any((b) => b.gainDb > 0) &&
            eq.current.preampDb > eqAutoPreamp(settings: eq.current) + 0.05)
          Text(
            'Возможна перегрузка: нажмите «Авто» у предусилителя',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.tertiary,
            ),
          ),
      ],
    );
  }
}

String _modeName(EqModeDto m) => switch (m) {
  EqModeDto.graphic10 => '10 полос',
  EqModeDto.graphic18 => '18 полос',
  EqModeDto.parametric => 'параметр.',
};

class _PreampRow extends StatelessWidget {
  const _PreampRow({required this.eq});

  final EqController eq;

  @override
  Widget build(BuildContext context) {
    final db = eq.current.preampDb.clamp(-24.0, 12.0);
    return Row(
      children: [
        const SizedBox(width: 110, child: Text('Предусилитель')),
        Expanded(
          child: Slider(
            value: db,
            min: -24,
            max: 12,
            divisions: 72,
            onChanged: (v) => eq.setPreamp(v, apply: false),
            onChangeEnd: (_) => eq.commit(),
          ),
        ),
        SizedBox(
          width: 64,
          child: Text('${_formatDb(db)} дБ', textAlign: TextAlign.end),
        ),
        TextButton(onPressed: eq.autoPreamp, child: const Text('Авто')),
      ],
    );
  }
}

class _GraphicBands extends StatelessWidget {
  const _GraphicBands({required this.eq});

  final EqController eq;

  static const _range = 12.0;

  @override
  Widget build(BuildContext context) {
    final bands = eq.current.bands;
    final labelStyle = Theme.of(context).textTheme.labelSmall;
    return SizedBox(
      height: 300,
      child: LayoutBuilder(
        builder: (context, constraints) {
          final columnWidth = math.max(
            44.0,
            constraints.maxWidth / bands.length,
          );
          return ListView.builder(
            scrollDirection: Axis.horizontal,
            itemCount: bands.length,
            itemBuilder: (context, i) {
              final b = bands[i];
              final gain = b.gainDb.clamp(-_range, _range);
              return SizedBox(
                width: columnWidth,
                child: Column(
                  children: [
                    Text(_formatDb(gain), style: labelStyle),
                    Expanded(
                      child: RotatedBox(
                        quarterTurns: 3,
                        child: Slider(
                          value: gain,
                          min: -_range,
                          max: _range,
                          divisions: 48,
                          onChanged: (v) => eq.setBand(
                            i,
                            b.copyWith(gainDb: v),
                            apply: false,
                          ),
                          onChangeEnd: (_) => eq.commit(),
                        ),
                      ),
                    ),
                    Text(_formatFreq(b.freqHz), style: labelStyle),
                  ],
                ),
              );
            },
          );
        },
      ),
    );
  }
}

class _ParametricBands extends StatelessWidget {
  const _ParametricBands({required this.eq});

  final EqController eq;

  @override
  Widget build(BuildContext context) {
    final bands = eq.current.bands;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (bands.isEmpty)
          const Padding(
            padding: EdgeInsets.symmetric(vertical: 16),
            child: Text(
              'Полос нет. Добавьте полосу вручную или импортируйте пресет AutoEq.',
              textAlign: TextAlign.center,
            ),
          ),
        for (var i = 0; i < bands.length; i++)
          _BandEditor(key: ValueKey(i), index: i, band: bands[i], eq: eq),
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerLeft,
          child: OutlinedButton.icon(
            icon: const Icon(Icons.add),
            label: Text(
              'Добавить полосу (${bands.length}/${EqController.maxParametricBands})',
            ),
            onPressed: bands.length < EqController.maxParametricBands
                ? eq.addBand
                : null,
          ),
        ),
      ],
    );
  }
}

class _BandEditor extends StatelessWidget {
  const _BandEditor({
    super.key,
    required this.index,
    required this.band,
    required this.eq,
  });

  final int index;
  final EqBandDto band;
  final EqController eq;

  // Частота и добротность — по логарифмической шкале.
  static final _minF = math.log(20) / math.ln10;
  static final _maxF = math.log(20000) / math.ln10;
  static final _minQ = math.log(0.1) / math.ln10;
  static final _maxQ = math.log(10) / math.ln10;

  static double _log10(double v) => math.log(v) / math.ln10;

  static double _pow10(double v) => math.pow(10, v).toDouble();

  void _set(EqBandDto b, {bool apply = false}) =>
      eq.setBand(index, b, apply: apply);

  @override
  Widget build(BuildContext context) {
    final labelStyle = Theme.of(context).textTheme.bodySmall;
    Widget row(String label, Widget slider, String value) => Row(
      children: [
        SizedBox(width: 72, child: Text(label, style: labelStyle)),
        Expanded(child: slider),
        SizedBox(
          width: 72,
          child: Text(value, textAlign: TextAlign.end, style: labelStyle),
        ),
      ],
    );

    return Card(
      margin: const EdgeInsets.symmetric(vertical: 4),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 4, 4, 4),
        child: Column(
          children: [
            Row(
              children: [
                Text('Полоса ${index + 1}'),
                const SizedBox(width: 16),
                DropdownButton<FilterKindDto>(
                  value: band.kind,
                  underline: const SizedBox.shrink(),
                  items: const [
                    DropdownMenuItem(
                      value: FilterKindDto.peaking,
                      child: Text('Пиковый'),
                    ),
                    DropdownMenuItem(
                      value: FilterKindDto.lowShelf,
                      child: Text('НЧ-полка'),
                    ),
                    DropdownMenuItem(
                      value: FilterKindDto.highShelf,
                      child: Text('ВЧ-полка'),
                    ),
                  ],
                  onChanged: (k) {
                    if (k != null) _set(band.copyWith(kind: k), apply: true);
                  },
                ),
                const Spacer(),
                IconButton(
                  tooltip: 'Удалить полосу',
                  icon: const Icon(Icons.close),
                  onPressed: () => eq.removeBand(index),
                ),
              ],
            ),
            row(
              'Частота',
              Slider(
                value: _log10(band.freqHz.clamp(20, 20000)),
                min: _minF,
                max: _maxF,
                onChanged: (v) =>
                    _set(band.copyWith(freqHz: _pow10(v).roundToDouble())),
                onChangeEnd: (_) => eq.commit(),
              ),
              '${_formatFreq(band.freqHz)} Гц',
            ),
            row(
              'Усиление',
              Slider(
                value: band.gainDb.clamp(-24, 24),
                min: -24,
                max: 24,
                divisions: 96,
                onChanged: (v) => _set(band.copyWith(gainDb: v)),
                onChangeEnd: (_) => eq.commit(),
              ),
              '${_formatDb(band.gainDb)} дБ',
            ),
            row(
              'Q',
              Slider(
                value: _log10(band.q.clamp(0.1, 10)),
                min: _minQ,
                max: _maxQ,
                onChanged: (v) => _set(
                  band.copyWith(q: double.parse(_pow10(v).toStringAsFixed(2))),
                ),
                onChangeEnd: (_) => eq.commit(),
              ),
              band.q.toStringAsFixed(2),
            ),
          ],
        ),
      ),
    );
  }
}
