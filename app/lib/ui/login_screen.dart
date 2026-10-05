import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';

import '../src/rust/api/yandex.dart';
import 'errors.dart';

/// Вход по коду подтверждения: пароль вводится только на сайте Яндекса.
class LoginScreen extends StatefulWidget {
  const LoginScreen({super.key, required this.onLoggedIn});

  final VoidCallback onLoggedIn;

  @override
  State<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends State<LoginScreen> {
  DeviceCodeDto? _code;
  String? _error;
  bool _busy = false;

  Future<void> _login() async {
    setState(() {
      _busy = true;
      _error = null;
      _code = null;
    });
    try {
      final code = await startLogin();
      setState(() => _code = code);
      await launchUrl(Uri.parse(code.verificationUrl));
      await finishLogin();
      widget.onLoggedIn();
    } catch (e) {
      setState(() => _error = errorText(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final code = _code;
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 420),
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text('moth-amp', style: theme.textTheme.headlineMedium),
                const SizedBox(height: 32),
                if (code == null) ...[
                  FilledButton(
                    onPressed: _busy ? null : _login,
                    child: const Text('Войти через Яндекс'),
                  ),
                ] else ...[
                  const Text('Введите код на странице Яндекса:'),
                  const SizedBox(height: 12),
                  SelectableText(
                    code.userCode,
                    style: theme.textTheme.displaySmall?.copyWith(
                      letterSpacing: 4,
                      fontFamily: 'monospace',
                    ),
                  ),
                  const SizedBox(height: 12),
                  Wrap(
                    spacing: 8,
                    children: [
                      OutlinedButton(
                        onPressed: () => Clipboard.setData(
                          ClipboardData(text: code.userCode),
                        ),
                        child: const Text('Скопировать код'),
                      ),
                      OutlinedButton(
                        onPressed: () =>
                            launchUrl(Uri.parse(code.verificationUrl)),
                        child: const Text('Открыть страницу'),
                      ),
                    ],
                  ),
                  const SizedBox(height: 24),
                  const LinearProgressIndicator(),
                  const SizedBox(height: 8),
                  Text(
                    'Жду подтверждения (код действует ${code.expiresIn ~/ 60} мин)',
                    style: theme.textTheme.bodySmall,
                  ),
                ],
                if (_error != null) ...[
                  const SizedBox(height: 24),
                  Text(
                    _error!,
                    style: TextStyle(color: theme.colorScheme.error),
                  ),
                  const SizedBox(height: 8),
                  TextButton(
                    onPressed: _login,
                    child: const Text('Попробовать снова'),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
