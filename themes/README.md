# Piuma themes

Colour-only piuma config fragments, one file per theme. The setup's theme
picker applies a file over the **current** config: it copies the colour keys
under `panel` and leaves sections, panels, sensors and layout untouched.

## Format

```json
{
  "name": "Bianco su nero",
  "panel": {
    "theme_mode": "dark",
    "wallpaper_palette": false,
    "default_bg_color": "#333333ff",
    "default_text_color": "#ffffffff",
    "sensor_defaults": {
      "text_color": "#ffffffff",
      "text_color_hover": "#ffffffff",
      "bg_color": "#3d3d3dff",
      "bg_color_hover": "#4d4d4dff",
      "border_color": "#5f5f5fff",
      "border_color_hover": "#8a8a8aff"
    },
    "section_defaults": {
      "bg_color": "#2b2b2bff",
      "text_color": "#ffffffff",
      "border_color": "#5f5f5fff"
    }
  }
}
```

- `name` is the label the picker shows; the bar ignores it.
- `wallpaper_palette` is always `false`: a theme must win over the colours
  extracted from the wallpaper.
- Colours are `#rrggbb` or `#rrggbbaa`; `theme_mode` is `dark` or `light`.
- A section or sensor with its own explicit colour still wins: the theme
  only fills the defaults.

## Themes

| File | Name | Notes |
|---|---|---|
| `01-bianco-su-nero.json` | Bianco su nero | `#333333` background |
| `02-nero-su-bianco.json` | Nero su bianco | `#eeeeee` background |
| `03-nero-su-arancione.json` | Nero su arancione | muted orange |
| `04-bianco-su-verde.json` | Bianco su verde | muted green |
| `05-ciano-su-blu-scuro.json` | Ciano su blu scuro | cyan on navy |
| `06-dracula.json` | Dracula | official Dracula palette |
| `07-catppuccin-mocha.json` | Catppuccin Mocha | official Mocha palette |
| `08-gruvbox-dark.json` | Gruvbox Dark | official Gruvbox palette |
| `09-nord.json` | Nord | official Nord palette |
| `10-tokyo-night.json` | Tokyo Night | navy/blue palette |

`cargo test --bin piuma shipped_themes` parses every file and checks the
bar and pill contrasts meet WCAG 4.5:1.
