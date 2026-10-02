# Piuma GUI Configuration Tool — Implemented

## Status: Complete

`piuma-setup` is a working GUI built with the `iced` framework (same dependency Piuma already uses). It compiles cleanly and can load, edit, and save Piuma's JSON configuration.

## Build & Run

```bash
cargo build --release --bin piuma-setup
./target/release/piuma-setup
```

The binary is declared in `Cargo.toml`:

```toml
[[bin]]
name = "piuma-setup"
path = "src/bin/piuma-setup.rs"
```

## What it does

- **Panel settings** — font, font size, section layout, default section weight, default background/text color, anchor, align, percentage.
- **Sections** — add/remove, name, layout, weight, background/text color.
- **Sensors** — add/remove, name, description, source type, source-specific config (interface / device / format / command / env var / static text), and all optional styling: text & background color, hover variants, border color & width (with hover), border radius, padding.
- **Load / Save / Reset** — Load reads `~/.config/piuma/piuma.json`, Save writes it, Reset restores built-in defaults.

The GUI shows every optional property (including hover colors) even when omitted in the JSON, so you can enable them visually.

## Schema correctness

The GUI targets `src/config.rs` exactly:

- Output uses `panel` → `PanelConfig` → `SectionConfig` → `SensorConfig`, matching the real `Config` struct.
- Source type is serialized with the tagged form `"source": { "type": "..." }`.
- `StaticText` serializes as `{ "type": "StaticText", "static_text": "..." }` — matching the `SensorSource::StaticText { static_text }` variant.
- `Clock` accepts an optional `format`; `Network`/`Audio` use `interface`/`device`.

A saved config is directly loadable by `piuma` itself via `piuma --config ~/.config/piuma/piuma.json`.

## Notes

- The window title is "Piuma Setup"; it closes on window-close by default.
- Colors are free-form strings (`#RRGGBB` hex or `r g b a` floats), parsed by Piuma's `parse_color` at runtime.
- The build emits only harmless lifetime-elision warnings from the `iced::text` API.
