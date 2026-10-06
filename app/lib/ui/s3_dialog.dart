import 'package:flutter/material.dart';

import '../src/rust/api/s3.dart';
import 'errors.dart';

/// Подключение S3-совместимого хранилища для своей музыки.
/// Возвращает `true`, если подключение прошло.
Future<bool> showS3Dialog(BuildContext context) async =>
    await showDialog<bool>(
      context: context,
      builder: (_) => const _S3Dialog(),
    ) ??
    false;

class _S3Dialog extends StatefulWidget {
  const _S3Dialog();

  @override
  State<_S3Dialog> createState() => _S3DialogState();
}

class _S3DialogState extends State<_S3Dialog> {
  final _endpoint = TextEditingController(
    text: 'https://storage.yandexcloud.net',
  );
  final _region = TextEditingController(text: 'ru-central1');
  final _bucket = TextEditingController();
  final _accessKey = TextEditingController();
  final _secretKey = TextEditingController();
  bool _pathStyle = true;
  bool _connected = false;
  bool _busy = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    // Если хранилище уже подключено — подставляем настройки (кроме секрета).
    s3Status()
        .then((status) {
          if (!mounted || !status.connected) return;
          setState(() {
            _connected = true;
            _endpoint.text = status.endpoint;
            _region.text = status.region;
            _bucket.text = status.bucket;
            _accessKey.text = status.accessKey;
            _pathStyle = status.pathStyle;
          });
        })
        .catchError((Object _) {});
  }

  @override
  void dispose() {
    for (final c in [_endpoint, _region, _bucket, _accessKey, _secretKey]) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _connect() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final count = await s3Connect(
        config: S3ConfigDto(
          endpoint: _endpoint.text.trim(),
          region: _region.text.trim(),
          bucket: _bucket.text.trim(),
          accessKey: _accessKey.text.trim(),
          secretKey: _secretKey.text.trim(),
          pathStyle: _pathStyle,
        ),
      );
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('S3 подключено, треков в библиотеке: $count')),
      );
      Navigator.pop(context, true);
    } catch (e) {
      if (mounted) setState(() => _error = errorText(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _disconnect() async {
    await s3Disconnect();
    if (mounted) Navigator.pop(context, true);
  }

  Widget _field(
    TextEditingController c,
    String label, {
    String? hint,
    bool secret = false,
  }) => Padding(
    padding: const EdgeInsets.only(bottom: 10),
    child: TextField(
      controller: c,
      obscureText: secret,
      enableSuggestions: !secret,
      autocorrect: false,
      decoration: InputDecoration(
        labelText: label,
        hintText: hint,
        border: const OutlineInputBorder(),
        isDense: true,
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return AlertDialog(
      title: const Text('Хранилище S3 для своей музыки'),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                'Любое S3-совместимое хранилище: Yandex Object Storage, Selectel, '
                'VK Cloud, Cloudflare R2 или свой сервер. Файлы лежат в бакете под '
                'префиксом moth-amp/; если бакета нет, он создастся сам. Ключи '
                'хранятся в системном хранилище. Свой NAS — инструкция в '
                'репозитории: docs/s3-nas.md.',
                style: theme.textTheme.bodySmall,
              ),
              const SizedBox(height: 12),
              _field(
                _endpoint,
                'Адрес (endpoint)',
                hint: 'https://storage.yandexcloud.net',
              ),
              _field(_region, 'Регион', hint: 'ru-central1'),
              _field(_bucket, 'Бакет'),
              _field(_accessKey, 'Ключ доступа (Access key)'),
              _field(
                _secretKey,
                _connected
                    ? 'Секретный ключ (введите заново, чтобы изменить)'
                    : 'Секретный ключ',
                secret: true,
              ),
              SwitchListTile(
                contentPadding: EdgeInsets.zero,
                title: const Text('Адреса в стиле path'),
                subtitle: const Text(
                  'endpoint/бакет/файл — нужно многим своим серверам',
                ),
                value: _pathStyle,
                onChanged: (v) => setState(() => _pathStyle = v),
              ),
              if (_error != null)
                Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            ],
          ),
        ),
      ),
      actions: [
        if (_connected)
          TextButton(
            onPressed: _busy ? null : _disconnect,
            child: const Text('Отключить'),
          ),
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Отмена'),
        ),
        FilledButton(
          onPressed: _busy ? null : _connect,
          child: _busy
              ? const SizedBox.square(
                  dimension: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Подключить'),
        ),
      ],
    );
  }
}
