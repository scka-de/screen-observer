# screen-observer

A trait-based Rust abstraction for desktop screen observation with pluggable backends.

## Overview

`screen-observer` defines a `ScreenObserver` trait and common types (`ObservationEvent`, `WindowContext`) for consuming screen activity events. Backends implement the trait to provide real or mock observation data.

## Backends

- **Mock** — configurable fake events for testing and demos
- **Screenpipe** — adapter for [Screenpipe](https://github.com/screenpipe/screenpipe) (V0)
- **Native** — direct OS API integration (planned)

## Status

Pre-alpha. Under active development.

## License

MIT
