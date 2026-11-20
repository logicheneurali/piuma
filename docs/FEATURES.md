# Piuma — features

[← Back to the README](../README.md)

## Wallpaper

`panel.wallpaper` draws the desktop background in-process. `mode` is
`image` (the default) or `color`: an image uses a cover fit — it fills
each output, keeps its aspect ratio and crops the overhang, so different
aspect ratios crop differently per monitor — while `color` paints a
solid colour. Both are selected in `piuma-setup`'s Wallpaper tab, which
also has a switch for Hyprland's own 8K default wallpaper/logo (it edits
the `misc.disable_hyprland_logo` and `misc.force_default_wallpaper`
values in the Hyprland config and reloads it).

## Themes

Ten colour themes ship inside the binary (no files to install): white on
black, black on white, black on muted orange, white on muted green, cyan
on navy, purple on graphite, lavender on indigo, amber on charcoal, ice
on slate and blue on midnight.
A theme is a colour-only fragment — bar background/text, sensor defaults
and section defaults — applied **over** the user's config in memory;
sections, panels, sensors and layout are untouched, and an explicit
per-sensor or per-section colour still wins.

The active theme is saved in its own file,
`$XDG_CONFIG_HOME/piuma/theme.json` (usually
`~/.config/piuma/theme.json`):

```json
{ "theme": "purple-on-graphite" }
```

**`piuma.json` is never rewritten** by a theme switch: the bar reads
`theme.json` at startup and watches it while running, so picking a theme
in `piuma-setup`'s *Themes* tab or in the `theme` sensor applies it live.
Absent, `null` or an unknown slug means *Custom* — the user's own
colours. A theme suspends `wallpaper_palette` in memory (the wallpaper's
tones would otherwise repaint it); switching back to Custom restores it.

The fragment format (also documented in `themes/README.md`):

```json
{
  "name": "Purple on graphite",
  "panel": {
    "theme_mode": "dark",
    "wallpaper_palette": false,
    "default_bg_color": "#282a36ff",
    "default_text_color": "#f8f8f2ff",
    "sensor_defaults": { "text_color": "#f8f8f2ff", "bg_color": "#44475aff", "...": "..." },
    "section_defaults": { "bg_color": "#21222cff", "...": "..." }
  }
}
```

## Dashboard

The `dashboard` sensor opens a full-screen command center: a dimmed
scrim with a centred two-column card. The header carries the clock and
date plus battery and uptime; the left column has the playing track
with its local cover art, transport, a position bar, the stateful
controls (power profile with the current profile, Caffeine, Bluetooth,
Lock) and the volume slider — brightness too when `brightnessctl` can
write it; the right column shows every numeric sensor as a tile with its
value and sparkline, then the scrollable notification list. Each control
displays its state and updates immediately (the power profile shows the
next profile at once and the real one on the next probe; the volume and
brightness sliders write on release). When the notification list is
empty the sensor tiles stretch to fill the card. A click outside the
card, the ✕ or the sensor again closes it. The probe (MPRIS, profile,
volume, brightness, Bluetooth, network, uptime) and the cover decode run
only while the dashboard is open: a closed dashboard costs nothing — no
polling, no extra D-Bus traffic.

## Sensor Types

Sensor `type` values follow a naming convention by category:

- **`perc_`** – percentage: `perc_cpu`, `perc_gpu`, `perc_mem`, `perc_swap`,
  `perc_disk`, `perc_battery`, `perc_backlight`, `perc_volume`. Each sensor
  can pick `value_display`: `numeric` (the number), `grid` (a 3×3 grid,
  one full square per 10%, lit left→right and bottom→top; a remainder over
  5% adds the same 4×4 square at a lighter alpha in the next cell, so 35% lights the bottom
  row and 36% adds the partial marker) or `both` (the default). The same choice
  covers the `used_mem`/`used_swap`/`used_disk`/`used_battery` family, whose
  grid is the fraction of the current total, and the network rates, whose
  reference is `scale_max` (`"12M"`, `"1.5G"`, `"500K"`, `"1T"`; binary
  tiers, bare = B/s; absent = the peak of the last two minutes). The
  temperature sensors (`temp_cpu`, `temp_gpu`) use the same grid with a
  conventional 0–100 °C scale (one square per 10 °C), still coloured by
  their `color_rules`
- **`used_`** – numeric consumption: `used_mem`, `used_swap`, `used_disk`,
  `used_battery`, `used_netdown`, `used_netup`
- **`total_`** – full value: `total_mem`, `total_swap`, `total_disk`,
  `total_battery`
- **`detail_`** – used/total ratio: `detail_mem`, `detail_swap`, `detail_disk`
- **`temp_`** – temperature: `temp_cpu`, `temp_gpu`
- **no prefix** – not numeric: `net_state`, `uptime`, `clock`, `window`,
  `workspace`, `workspace_dots`, `command`, `env_var`, `static_text` (Text/Icon),
  `volume_meter`, `media`, `tray`, `notifications`, `taskbar`, `layout`,
  `lock`, `caffeine`, `power`, `theme`, `dashboard`, `wifi`

The old names (`ProcCpu`, `NvidiaGpu`, `ProcMem`, `ProcDisk`, `Audio`,
`Network`, `Uptime`, `Clock`, `Window`, `Workspace`, `Command`, `EnvVar`,
`StaticText`) are still accepted, so existing configs keep loading;
`piuma-setup` converts them to the canonical names on load, so the next
save writes the new names into `piuma.json`.

## Desktop Notifications

- **Freedesktop server**: with the `panel.notifications` block present,
  piuma claims `org.freedesktop.Notifications` on the session bus — apps
  notify over D-Bus just like with mako/dunst
- **Popup stack**: cards on the overlay layer in the corner opposite the
  bar (icon from `image-data`/`image-path`/`app_icon`, summary and
  markup-stripped body), up to 5 at once; clicking a card runs the
  notification's `default` action and dismisses it, the ✕ only dismisses;
  the card look (font size, colors incl. hover, borders, padding) is
  configurable — white text on black, grey border by default
- **Bar sensor**: the `notifications` source shows the newest active
  summary with a count; the wheel walks the history (`(selected/total)`),
  the click runs the shown entry's `default` action
- **Spec signals**: `NotificationClosed` uses reason 1 (expired),
  2 (dismissed) and 3 (`CloseNotification`); `ActionInvoked` carries the
  invoked action key

## Styling Features

- **Hover Effects**: Color changes on mouse hover
- **Border Styling**: Customizable borders with radius and width
- **Padding Control**: Space around sensor content
- **Section-level Styling**: Background and text colors per section
- **Collapsible sections**: `"collapsible": true` on a section adds a
  clickable dot in the section's own text colour; clicking it collapses
  the section to the dot and back. Runtime state only: the bar starts
  with every section expanded, and a collapsed section's popups close.
- **Rounded Screen Corners**: `panel.screen_corners { enable, color, radius }`
  masks each display corner with a solid-colour square minus the inner
  quarter disk, so the screen itself reads as rounded like the panel —
  drawn on layer-shell overlay surfaces, so it needs a layer-shell
  compositor (Hyprland: works). The controls live in piuma-setup's
  *Tweaks* tab; the JSON block stays under `panel.screen_corners`.

## Panel Positioning

- **Anchor Points**: top, bottom, left, right (supports combinations)
- **Alignment Options**: left, center, right (top/bottom on vertical panels)
- **Fill Mode**: `align: "fill"` spreads the sections across the whole panel
  with equal spacers, so the panel always spans 100% of the screen; with
  exactly 3 sections the middle one is kept centered when there is enough
  room (otherwise it falls back to the equal-spacer distribution)
- **Percentage-based filling**: Configure percentage of available space
