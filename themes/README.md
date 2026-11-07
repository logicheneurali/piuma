# Piuma themes

Colour-only piuma config fragments, one file per theme, embedded in the
binary. The setup's **Themes** tab and the `theme` sensor apply a file over
the bar's config **in memory** (sections, panels, sensors and layout stay
untouched) and save only the active slug in
`~/.config/piuma/theme.json`; `piuma.json` is never rewritten.

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
| `06-viola-su-grafite.json` | Viola su grafite | graphite background, purple accent |
| `07-lavanda-su-indaco.json` | Lavanda su indaco | indigo background, lavender/blue accents |
| `08-ambra-su-carbone.json` | Ambra su carbone | charcoal background, amber accent |
| `09-ghiaccio-su-ardesia.json` | Ghiaccio su ardesia | slate background, ice-blue accent |
| `10-blu-su-mezzanotte.json` | Blu su mezzanotte | midnight-blue background, periwinkle/blue accents |

`cargo test --bin piuma shipped_themes` parses every file and checks the
bar and pill contrasts meet WCAG 4.5:1.
