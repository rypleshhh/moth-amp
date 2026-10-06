import 'package:flutter/material.dart';

/// Тёплая ночь + свет лампы. Без лишнего лоска: моноширинные подписи
/// строчными, тонкие линии, янтарь только там, где что-то происходит.
abstract final class Moth {
  static const night = Color(0xFF131218);
  static const surface = Color(0xFF18171E);
  static const raised = Color(0xFF1F1E26);
  static const line = Color(0xFF2A2932);
  static const amber = Color(0xFFEDB04A);
  static const paper = Color(0xFFEDE6DA);
  static const dim = Color(0xFF8E887F);
  static const mono = 'PlexMono';

  /// Подписи и заголовки «как в заметках»: моно, строчные, чуть разрежено.
  static const label = TextStyle(
    fontFamily: mono,
    fontSize: 12,
    letterSpacing: 0.4,
    color: dim,
  );
}

ThemeData buildMothTheme() {
  final scheme =
      ColorScheme.fromSeed(
        seedColor: Moth.amber,
        brightness: Brightness.dark,
      ).copyWith(
        primary: Moth.amber,
        onPrimary: Moth.night,
        surface: Moth.surface,
        onSurface: Moth.paper,
        onSurfaceVariant: Moth.dim,
        surfaceContainer: Moth.raised,
        surfaceContainerHighest: Moth.line,
        outline: Moth.line,
        outlineVariant: Moth.line,
      );

  final base = ThemeData(colorScheme: scheme, useMaterial3: true);
  return base.copyWith(
    scaffoldBackgroundColor: Moth.night,
    dividerColor: Moth.line,
    dividerTheme: const DividerThemeData(
      color: Moth.line,
      space: 1,
      thickness: 1,
    ),
    appBarTheme: const AppBarTheme(
      backgroundColor: Moth.night,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      titleSpacing: 16,
    ),
    tabBarTheme: const TabBarThemeData(
      labelColor: Moth.paper,
      unselectedLabelColor: Moth.dim,
      labelStyle: TextStyle(fontFamily: Moth.mono, fontSize: 13),
      unselectedLabelStyle: TextStyle(fontFamily: Moth.mono, fontSize: 13),
      indicatorSize: TabBarIndicatorSize.label,
      indicator: UnderlineTabIndicator(
        borderSide: BorderSide(color: Moth.amber, width: 2),
      ),
      dividerColor: Moth.line,
      overlayColor: WidgetStatePropertyAll(Color(0x10EDB04A)),
    ),
    listTileTheme: const ListTileThemeData(
      dense: true,
      selectedColor: Moth.amber,
      selectedTileColor: Color(0x0FEDB04A),
      iconColor: Moth.dim,
      contentPadding: EdgeInsets.symmetric(horizontal: 16),
    ),
    sliderTheme: base.sliderTheme.copyWith(
      trackHeight: 3,
      inactiveTrackColor: Moth.line,
      thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 7),
      overlayShape: const RoundSliderOverlayShape(overlayRadius: 14),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 14),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        side: const BorderSide(color: Moth.line),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
    ),
    segmentedButtonTheme: SegmentedButtonThemeData(
      style: SegmentedButton.styleFrom(
        side: const BorderSide(color: Moth.line),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
        selectedBackgroundColor: const Color(0x22EDB04A),
        selectedForegroundColor: Moth.amber,
      ),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: Moth.surface,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(8),
        side: const BorderSide(color: Moth.line),
      ),
    ),
    snackBarTheme: const SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      backgroundColor: Moth.raised,
      contentTextStyle: TextStyle(color: Moth.paper),
    ),
    tooltipTheme: const TooltipThemeData(
      textStyle: TextStyle(
        fontFamily: Moth.mono,
        fontSize: 11,
        color: Moth.paper,
      ),
      decoration: BoxDecoration(color: Moth.raised),
    ),
  );
}

/// Пустое состояние без казённости.
class EmptyNote extends StatelessWidget {
  const EmptyNote(
    this.text, {
    super.key,
    this.icon = Icons.nightlight_outlined,
  });

  final String text;
  final IconData icon;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, color: Moth.line, size: 40),
            const SizedBox(height: 10),
            Text(text, textAlign: TextAlign.center, style: Moth.label),
          ],
        ),
      ),
    );
  }
}
