import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';

import '../player/player_controller.dart';
import '../src/rust/api/eq.dart';

extension EqSettingsCopy on EqSettingsDto {
  EqSettingsDto copyWith({
    bool? enabled,
    EqModeDto? mode,
    double? preampDb,
    List<EqBandDto>? bands,
  }) =>
      EqSettingsDto(
        enabled: enabled ?? this.enabled,
        mode: mode ?? this.mode,
        preampDb: preampDb ?? this.preampDb,
        bands: bands ?? this.bands,
      );
}

extension EqBandCopy on EqBandDto {
  EqBandDto copyWith({
    FilterKindDto? kind,
    double? freqHz,
    double? gainDb,
    double? q,
  }) =>
      EqBandDto(
        kind: kind ?? this.kind,
        freqHz: freqHz ?? this.freqHz,
        gainDb: gainDb ?? this.gainDb,
        q: q ?? this.q,
      );
}

/// Состояние эквалайзера: текущие настройки, пользовательские пресеты,
/// применение к плееру и сохранение на диск.
///
/// Изменения во время перетаскивания ползунка (`apply: false`) только
/// перерисовывают UI; фильтры mpv пересобираются по отпусканию ползунка,
/// иначе звук бы прерывался.
class EqController extends ChangeNotifier {
  EqController(this._player);

  static const maxParametricBands = 20;

  final PlayerController _player;
  String? _path;
  Timer? _saveTimer;

  EqSettingsDto current = eqFlat(mode: EqModeDto.graphic10);
  List<EqPresetDto> userPresets = [];

  /// Название выбранного пресета; сбрасывается при ручной правке.
  String? presetName;

  Future<void> load() async {
    final dir = await getApplicationSupportDirectory();
    _path = '${dir.path}${Platform.pathSeparator}equalizer.json';
    try {
      final state = await eqLoad(path: _path!);
      current = state.current;
      userPresets = List.of(state.userPresets);
    } catch (e) {
      debugPrint('Не удалось прочитать настройки эквалайзера: $e');
    }
    notifyListeners();
    await _apply();
  }

  List<EqPresetDto> get builtinPresets => eqBuiltinPresets(mode: current.mode);

  bool get isGraphic => current.mode != EqModeDto.parametric;

  void _update(EqSettingsDto s, {bool apply = true, String? preset}) {
    current = s;
    presetName = preset;
    notifyListeners();
    if (apply) {
      _apply();
      _scheduleSave();
    }
  }

  /// Применить текущие настройки (например, по отпусканию ползунка).
  void commit() {
    _apply();
    _scheduleSave();
  }

  void setEnabled(bool v) =>
      _update(current.copyWith(enabled: v), preset: presetName);

  void setMode(EqModeDto mode) {
    if (mode == current.mode) return;
    _update(eqWithMode(settings: current, mode: mode));
  }

  void setPreamp(double db, {bool apply = true}) =>
      _update(current.copyWith(preampDb: db), apply: apply);

  void autoPreamp() =>
      _update(current.copyWith(preampDb: eqAutoPreamp(settings: current)));

  void setBand(int index, EqBandDto band, {bool apply = true}) {
    final bands = List.of(current.bands)..[index] = band;
    _update(current.copyWith(bands: bands), apply: apply);
  }

  void addBand() {
    if (current.bands.length >= maxParametricBands) return;
    final bands = List.of(current.bands)
      ..add(const EqBandDto(
        kind: FilterKindDto.peaking,
        freqHz: 1000,
        gainDb: 0,
        q: 1,
      ));
    _update(current.copyWith(bands: bands));
  }

  void removeBand(int index) {
    final bands = List.of(current.bands)..removeAt(index);
    _update(current.copyWith(bands: bands));
  }

  void reset() => _update(eqFlat(mode: current.mode));

  void applyPreset(EqPresetDto preset) => _update(
        preset.settings.copyWith(enabled: true),
        preset: preset.name,
      );

  /// Бросает исключение, если текст не похож на пресет AutoEq.
  void importAutoEq(String text) =>
      _update(eqParseAutoeq(text: text), preset: 'AutoEq');

  void saveAsPreset(String name) {
    userPresets
      ..removeWhere((p) => p.name == name)
      ..add(EqPresetDto(name: name, settings: current));
    presetName = name;
    notifyListeners();
    _scheduleSave();
  }

  void deletePreset(String name) {
    userPresets.removeWhere((p) => p.name == name);
    if (presetName == name) presetName = null;
    notifyListeners();
    _scheduleSave();
  }

  Future<void> _apply() async {
    await _player.setAudioFilter(eqToFilter(settings: current));
    await _player.setPreampDb(eqEffectivePreamp(settings: current));
  }

  void _scheduleSave() {
    final path = _path;
    if (path == null) return;
    _saveTimer?.cancel();
    _saveTimer = Timer(const Duration(milliseconds: 500), () {
      eqSave(
        path: path,
        state: EqStateDto(current: current, userPresets: List.of(userPresets)),
      ).catchError((Object e) {
        debugPrint('Не удалось сохранить эквалайзер: $e');
      });
    });
  }

  @override
  void dispose() {
    _saveTimer?.cancel();
    super.dispose();
  }
}
