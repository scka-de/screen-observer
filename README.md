# screen-observer

A trait-based Rust abstraction for desktop screen observation with pluggable backends.

## Overview

`screen-observer` defines a `ScreenObserver` trait and common types (`ObservationEvent`, `WindowContext`) for consuming screen activity events. Backends implement the trait to provide real or mock observation data.

## Quick Start

```rust
use screen_observer::create_observer;

let mut observer = create_observer();
let mut rx = observer.subscribe();
observer.start().await?;

while let Ok(event) = rx.recv().await {
    println!("{}: {}", event.window.app_name, event.ocr_text);
}
```

`create_observer()` automatically selects the best backend for the current platform.

## Backends

- **Native** (macOS) — direct screen capture via ScreenCaptureKit + OCR via Apple Vision framework. Requires the `native` feature and Screen Recording permission.
- **Screenpipe** — adapter for [Screenpipe](https://github.com/screenpipe/screenpipe) via HTTP polling.
- **Mock** — configurable fake events for testing and demos.

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `native` | off | Enables native macOS backend (screen capture + Vision OCR) |

## Requirements

### Native backend (macOS)

- macOS 12.3+ (for ScreenCaptureKit)
- Screen Recording permission (prompted on first use)
- No external dependencies — uses Apple system frameworks only

## License

MIT
