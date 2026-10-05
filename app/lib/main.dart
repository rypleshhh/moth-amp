import 'dart:io';

import 'package:flutter/material.dart';
import 'package:media_kit/media_kit.dart';
import 'package:path_provider/path_provider.dart';

import 'audio/downloads.dart';
import 'audio/eq_controller.dart';
import 'player/media_controls.dart';
import 'player/player_controller.dart';
import 'src/rust/api/cache.dart';
import 'src/rust/api/yandex.dart';
import 'src/rust/frb_generated.dart';
import 'ui/home_screen.dart';
import 'ui/login_screen.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  MediaKit.ensureInitialized();
  await RustLib.init();
  try {
    final dir = await getApplicationSupportDirectory();
    await cacheInit(
      dir: '${dir.path}${Platform.pathSeparator}cache',
      defaultLimitMb: 2048,
    );
  } catch (e) {
    // Без кэша приложение работает, треки просто играют напрямую.
    debugPrint('Кэш не открылся: $e');
  }
  final downloads = DownloadController();
  downloads.refresh();
  downloads.backfillMeta();
  runApp(MusicApp(downloads: downloads));
}

class MusicApp extends StatelessWidget {
  const MusicApp({super.key, required this.downloads});

  final DownloadController downloads;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'moth-amp',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFFEDB04A),
          brightness: Brightness.dark,
        ),
      ),
      // Над навигатором, чтобы загрузки были видны и в открытых поверх экранах.
      builder: (context, child) =>
          DownloadsScope(controller: downloads, child: child!),
      home: const RootScreen(),
    );
  }
}

/// Выбирает экран входа или основной экран.
class RootScreen extends StatefulWidget {
  const RootScreen({super.key});

  @override
  State<RootScreen> createState() => _RootScreenState();
}

class _RootScreenState extends State<RootScreen> {
  final _player = PlayerController();
  late final _eq = EqController(_player);
  late final Future<MediaControls?> _mediaControls = MediaControls.attach(
    _player,
  );
  late Future<bool> _loggedIn = isLoggedIn();

  void _refresh() => setState(() => _loggedIn = isLoggedIn());

  DownloadController? _downloads;
  String? _lastTrackId;

  /// Сменился трек — предыдущий мог сохраниться в кэш сам.
  void _onPlayerChanged() {
    final id = _player.current?.id;
    if (id != _lastTrackId) {
      _lastTrackId = id;
      _downloads?.refresh();
    }
  }

  @override
  void initState() {
    super.initState();
    // Подключаем медиаклавиши сразу, не дожидаясь первого build.
    _mediaControls.ignore();
    _eq.load();
    _player.addListener(_onPlayerChanged);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _downloads = DownloadsScope.of(context);
  }

  @override
  void dispose() {
    _player.removeListener(_onPlayerChanged);
    _mediaControls.then((c) => c?.dispose());
    _eq.dispose();
    _player.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<bool>(
      future: _loggedIn,
      builder: (context, snap) {
        if (!snap.hasData) {
          return const Scaffold(
            body: Center(child: CircularProgressIndicator()),
          );
        }
        if (snap.data!) {
          return HomeScreen(player: _player, eq: _eq, onLogout: _refresh);
        }
        return LoginScreen(onLoggedIn: _refresh);
      },
    );
  }
}
