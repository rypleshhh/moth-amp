import 'package:flutter/material.dart';
import 'package:media_kit/media_kit.dart';

import 'player/player_controller.dart';
import 'src/rust/api/yandex.dart';
import 'src/rust/frb_generated.dart';
import 'ui/home_screen.dart';
import 'ui/login_screen.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  MediaKit.ensureInitialized();
  await RustLib.init();
  runApp(const MusicApp());
}

class MusicApp extends StatelessWidget {
  const MusicApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'moth-amp',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF3FA34D),
          brightness: Brightness.dark,
        ),
      ),
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
  late Future<bool> _loggedIn = isLoggedIn();

  void _refresh() => setState(() => _loggedIn = isLoggedIn());

  @override
  void dispose() {
    _player.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<bool>(
      future: _loggedIn,
      builder: (context, snap) {
        if (!snap.hasData) {
          return const Scaffold(body: Center(child: CircularProgressIndicator()));
        }
        if (snap.data!) {
          return HomeScreen(player: _player, onLogout: _refresh);
        }
        return LoginScreen(onLoggedIn: _refresh);
      },
    );
  }
}
