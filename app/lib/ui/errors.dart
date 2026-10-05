import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;

/// Короткий текст ошибки для интерфейса: только первая строка, без цепочки
/// причин и трассировки стека, которые добавляет мост к Rust.
String errorText(Object error) {
  final text = error is AnyhowException ? error.message : '$error';
  return text.trim().split('\n').first;
}
