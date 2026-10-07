import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../audio/eq_controller.dart';
import '../player/player_controller.dart';
import '../src/rust/api/eq.dart';
import 'player_bar.dart';
import 'quick_eq.dart';
import 'theme.dart';

/// Цвета деки поверх палитры [Moth]: панель, дисплей, блики и тени рамок.
abstract final class DeckColors {
  static const panel = Color(0xFF1C1D24);
  static const lcd = Color(0xFF0C0D11);
  static const amberDim = Color(0x66EDB04A);
  static const light = Color(0x1FFFFFFF);
  static const shadow = Color(0x99000000);
}

/// Выпуклая (или вдавленная) панель в духе классических десктопных плееров.
class Bevel extends StatelessWidget {
  const Bevel({
    super.key,
    required this.child,
    this.inset = false,
    this.color = DeckColors.panel,
    this.padding = const EdgeInsets.all(8),
  });

  final Widget child;
  final bool inset;
  final Color color;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    final (tl, br) = inset
        ? (DeckColors.shadow, DeckColors.light)
        : (DeckColors.light, DeckColors.shadow);
    return Container(
      padding: padding,
      decoration: BoxDecoration(
        color: color,
        // Без скругления: Flutter не рисует скруглённую рамку с разными
        // цветами сторон, а именно они дают эффект объёма.
        border: Border(
          top: BorderSide(color: tl),
          left: BorderSide(color: tl),
          bottom: BorderSide(color: br),
          right: BorderSide(color: br),
        ),
      ),
      child: child,
    );
  }
}

/// Левая колонка широкого окна: дисплей, транспорт, обложка.
class ClassicDeck extends StatefulWidget {
  const ClassicDeck({super.key, required this.player, required this.eq});

  final PlayerController player;
  final EqController eq;

  @override
  State<ClassicDeck> createState() => _ClassicDeckState();
}

class _ClassicDeckState extends State<ClassicDeck> {
  /// Быстрый эквалайзер открыт поверх обложки.
  bool _eqOpen = false;

  void _toggleEq() => setState(() => _eqOpen = !_eqOpen);

  @override
  Widget build(BuildContext context) {
    final player = widget.player;
    final eq = widget.eq;
    return ColoredBox(
      color: DeckColors.panel,
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: ListenableBuilder(
          listenable: player,
          builder: (context, _) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Lcd(player: player, eq: eq),
              const SizedBox(height: 8),
              ProgressBar(player: player),
              _Transport(player: player, eq: eq, onEq: _toggleEq),
              const SizedBox(height: 12),
              Expanded(
                child: Stack(
                  fit: StackFit.expand,
                  children: [
                    _BigCover(url: player.current?.coverUrl),
                    if (_eqOpen) QuickEq(eq: eq, onClose: _toggleEq),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Lcd extends StatelessWidget {
  const _Lcd({required this.player, required this.eq});

  final PlayerController player;
  final EqController eq;

  @override
  Widget build(BuildContext context) {
    final track = player.current;
    final s = player.stream;
    final badges = <String>[
      if (s != null)
        '${s.codec.toUpperCase()}${s.bitrateKbps != null ? ' ${s.bitrateKbps}K' : ''}',
      if (s?.cached ?? false) 'КЭШ',
      if (s?.isPreview ?? false) 'ПРЕВЬЮ',
      if (player.waveActive) 'ВОЛНА',
    ];
    const small = TextStyle(
      fontFamily: Moth.mono,
      fontSize: 11,
      color: DeckColors.amberDim,
      letterSpacing: 1,
    );
    return Bevel(
      inset: true,
      color: DeckColors.lcd,
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Icon(
                player.loading
                    ? Icons.hourglass_empty
                    : player.playing
                    ? Icons.play_arrow
                    : Icons.pause,
                size: 18,
                color: Moth.amber,
              ),
              const SizedBox(width: 6),
              _TimeDisplay(player: player),
              const Spacer(),
              Flexible(
                child: Text(
                  badges.join(' · '),
                  textAlign: TextAlign.end,
                  maxLines: 2,
                  style: small,
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          _Marquee(
            // Ошибка воспроизведения важнее названия: на широком окне
            // нижней панели нет, и показать её больше негде.
            text: player.error != null
                ? '⚠ ${player.error}'
                : track == null
                ? 'moth-amp · ничего не играет'
                : '${track.artists} — ${track.title}',
            style: const TextStyle(
              fontFamily: Moth.mono,
              fontSize: 14,
              color: Moth.amber,
            ),
          ),
          const SizedBox(height: 10),
          SizedBox(height: 56, child: _EqCurve(eq: eq)),
        ],
      ),
    );
  }
}

/// Время: прошедшее или (по нажатию) оставшееся.
class _TimeDisplay extends StatefulWidget {
  const _TimeDisplay({required this.player});

  final PlayerController player;

  @override
  State<_TimeDisplay> createState() => _TimeDisplayState();
}

class _TimeDisplayState extends State<_TimeDisplay> {
  bool _remaining = false;

  String _fmt(Duration d) {
    final m = d.inMinutes.toString().padLeft(2, '0');
    final s = (d.inSeconds % 60).toString().padLeft(2, '0');
    return '$m:$s';
  }

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: () => setState(() => _remaining = !_remaining),
      child: StreamBuilder<Duration>(
        stream: widget.player.position,
        builder: (context, snap) {
          final pos = snap.data ?? Duration.zero;
          final total = widget.player.knownDuration;
          final shown = _remaining && total != null
              ? '-${_fmt(total - pos < Duration.zero ? Duration.zero : total - pos)}'
              : _fmt(pos);
          return Text(
            shown,
            style: const TextStyle(
              fontFamily: Moth.mono,
              fontSize: 34,
              height: 1,
              color: Moth.amber,
              fontFeatures: [FontFeature.tabularFigures()],
            ),
          );
        },
      ),
    );
  }
}

/// Бегущая строка в духе LCD: сдвиг на символ раз в 350 мс, только если
/// текст не помещается. Пошаговая прокрутка дешевле плавной анимации.
class _Marquee extends StatefulWidget {
  const _Marquee({required this.text, required this.style});

  final String text;
  final TextStyle style;

  @override
  State<_Marquee> createState() => _MarqueeState();
}

class _MarqueeState extends State<_Marquee> {
  Timer? _timer;
  int _offset = 0;

  @override
  void didUpdateWidget(_Marquee old) {
    super.didUpdateWidget(old);
    if (old.text != widget.text) _offset = 0;
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _ensureTimer(bool needed) {
    if (needed && _timer == null) {
      _timer = Timer.periodic(const Duration(milliseconds: 350), (_) {
        if (mounted) setState(() => _offset++);
      });
    } else if (!needed && _timer != null) {
      _timer!.cancel();
      _timer = null;
      _offset = 0;
    }
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final painter = TextPainter(
          text: TextSpan(text: 'M', style: widget.style),
          textDirection: TextDirection.ltr,
        )..layout();
        final fits = (constraints.maxWidth / painter.width).floor();
        final text = widget.text;
        final scroll = text.length > fits && fits > 0;
        // Таймер включаем/выключаем после кадра, не во время build.
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _ensureTimer(scroll);
        });
        var shown = text;
        if (scroll) {
          final loop = '$text   ·   ';
          final start = _offset % loop.length;
          shown = (loop + loop).substring(start, start + fits);
        }
        return Text(
          shown,
          maxLines: 1,
          softWrap: false,
          overflow: TextOverflow.clip,
          style: widget.style,
        );
      },
    );
  }
}

/// Кривая АЧХ эквалайзера (реальные данные из ядра, не имитация спектра).
class _EqCurve extends StatelessWidget {
  const _EqCurve({required this.eq});

  final EqController eq;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: eq,
      builder: (context, _) {
        final s = eq.current;
        return Stack(
          children: [
            Positioned.fill(
              child: CustomPaint(
                painter: _CurvePainter(
                  eqResponse(settings: s, points: 96),
                  s.enabled,
                ),
              ),
            ),
            Positioned(
              left: 0,
              top: 0,
              child: Text(
                s.enabled ? 'EQ' : 'EQ ВЫКЛ',
                style: const TextStyle(
                  fontFamily: Moth.mono,
                  fontSize: 9,
                  color: DeckColors.amberDim,
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}

class _CurvePainter extends CustomPainter {
  _CurvePainter(this.values, this.enabled);

  final Float32List values;
  final bool enabled;

  static const _rangeDb = 12.0;

  @override
  void paint(Canvas canvas, Size size) {
    final mid = size.height / 2;
    final grid = Paint()
      ..color = DeckColors.amberDim.withValues(alpha: 0.25)
      ..strokeWidth = 1;
    canvas.drawLine(Offset(0, mid), Offset(size.width, mid), grid);
    if (values.isEmpty) return;

    final path = Path();
    for (var i = 0; i < values.length; i++) {
      final x = size.width * i / (values.length - 1);
      final db = values[i].clamp(-_rangeDb, _rangeDb);
      final y = mid - db / _rangeDb * (mid - 2);
      i == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
    }
    final color = enabled ? Moth.amber : DeckColors.amberDim;
    final fill = Path.from(path)
      ..lineTo(size.width, mid)
      ..lineTo(0, mid)
      ..close();
    canvas.drawPath(fill, Paint()..color = color.withValues(alpha: 0.12));
    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5,
    );
  }

  @override
  bool shouldRepaint(_CurvePainter old) =>
      old.enabled != enabled || !_same(old.values, values);

  static bool _same(Float32List a, Float32List b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}

class _Transport extends StatelessWidget {
  const _Transport({
    required this.player,
    required this.eq,
    required this.onEq,
  });

  final PlayerController player;
  final EqController eq;
  final VoidCallback onEq;

  @override
  Widget build(BuildContext context) {
    final has = player.current != null;
    return Row(
      children: [
        _DeckButton(
          icon: Icons.skip_previous,
          onTap: has ? player.previous : null,
        ),
        const SizedBox(width: 6),
        _DeckButton(
          icon: player.playing ? Icons.pause : Icons.play_arrow,
          big: true,
          onTap: has ? player.playOrPause : null,
        ),
        const SizedBox(width: 6),
        _DeckButton(icon: Icons.skip_next, onTap: has ? player.next : null),
        const Spacer(),
        EqualizerButton(eq: eq, onPressed: onEq),
        SizedBox(width: 110, child: VolumeSlider(player: player)),
      ],
    );
  }
}

class _DeckButton extends StatelessWidget {
  const _DeckButton({required this.icon, this.onTap, this.big = false});

  final IconData icon;
  final VoidCallback? onTap;
  final bool big;

  @override
  Widget build(BuildContext context) {
    final size = big ? 48.0 : 40.0;
    return Opacity(
      opacity: onTap == null ? 0.4 : 1,
      child: GestureDetector(
        onTap: onTap,
        child: SizedBox.square(
          dimension: size,
          child: Bevel(
            padding: EdgeInsets.zero,
            child: Center(
              child: Icon(
                icon,
                size: big ? 28 : 22,
                color: big ? Moth.amber : null,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _BigCover extends StatelessWidget {
  const _BigCover({required this.url});

  final String? url;

  @override
  Widget build(BuildContext context) {
    final u = url;
    final placeholder = Bevel(
      inset: true,
      color: DeckColors.lcd,
      child: Center(
        child: Opacity(
          opacity: 0.35,
          child: Image.asset('assets/logo.png', width: 96, height: 96),
        ),
      ),
    );
    if (u == null) return placeholder;
    return LayoutBuilder(
      builder: (context, constraints) {
        final side = constraints.biggest.shortestSide;
        final px = (side * MediaQuery.devicePixelRatioOf(context)).round();
        return Align(
          alignment: Alignment.topCenter,
          child: ClipRRect(
            borderRadius: BorderRadius.circular(3),
            child: Image.network(
              // Обложка 400×400 из данных трека.
              u,
              width: side,
              height: side,
              fit: BoxFit.cover,
              cacheWidth: px,
              gaplessPlayback: true,
              errorBuilder: (_, _, _) => placeholder,
            ),
          ),
        );
      },
    );
  }
}
