# Piuma - System Information Panel for Hyprland

Piuma is a lightweight system information panel for Hyprland/Wayland, designed as a replacement for Sway/Waybar status bar functionality.

## Features

### Sensor Types

Sensor `type` values follow a naming convention by category:

- **`perc_`** – percentage: `perc_cpu`, `perc_gpu`, `perc_mem`, `perc_swap`,
  `perc_disk`, `perc_battery`, `perc_backlight`, `perc_volume`
- **`used_`** – numeric consumption: `used_mem`, `used_swap`, `used_disk`,
  `used_battery`, `used_netdown`, `used_netup`
- **`total_`** – full value: `total_mem`, `total_swap`, `total_disk`,
  `total_battery`
- **`detail_`** – used/total ratio: `detail_mem`, `detail_swap`, `detail_disk`
- **`temp_`** – temperature: `temp_cpu`
- **no prefix** – not numeric: `net_state`, `uptime`, `clock`, `window`,
  `workspace`, `command`, `env_var`, `static_text`, `tray`, `notifications`

The old names (`ProcCpu`, `NvidiaGpu`, `ProcMem`, `ProcDisk`, `Audio`,
`Network`, `Uptime`, `Clock`, `Window`, `Workspace`, `Command`, `EnvVar`,
`StaticText`) are still accepted, so existing configs keep loading;
`piuma-setup` converts them to the canonical names on load, so the next
save writes the new names into `piuma.json`.

### Desktop Notifications
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

### Styling Features
- **Hover Effects**: Color changes on mouse hover
- **Border Styling**: Customizable borders with radius and width
- **Padding Control**: Space around sensor content
- **Section-level Styling**: Background and text colors per section

### Panel Positioning
- **Anchor Points**: top, bottom, left, right (supports combinations)
- **Alignment Options**: left, center, right (top/bottom on vertical panels)
- **Fill Mode**: `align: "fill"` spreads the sections across the whole panel
  with equal spacers, so the panel always spans 100% of the screen; with
  exactly 3 sections the middle one is kept centered when there is enough
  room (otherwise it falls back to the equal-spacer distribution)
- **Percentage-based filling**: Configure percentage of available space

## Installation

1. Clone the repository:
```bash
git clone https://github.com/your-repo/piuma.git
cd piuma
```

2. Build the project:
```bash
cargo build --release
```

3. Run Piuma:
```bash
./target/release/piuma
```

## Configuration

Piuma uses JSON configuration files. The default configuration location is:
- `$XDG_CONFIG_HOME/piuma/piuma.json`
- `$HOME/.config/piuma/piuma.json`

Example configuration:
```json
{
  "panel": {
    "font": "Noto Sans",
    "font_size": 12,
    "bold": false,
    "section_layout": "horizontal",
    "default_section_weight": 1,
    "default_bg_color": "#1a1b26",
    "default_text_color": "#e8e8e8",
    "anchor": "bottom",
    "align": "center",
    "percentage": 0.8,
    "section_defaults": {
      "bg_color": "#23232f",
      "text_color": "#c8c8d4",
      "border_color": "#444466",
      "border_width": 1,
      "border_radius": 6,
      "border_type": "full"
    },
    "sections": [
      {
        "name": "System",
        "layout": "horizontal",
        "weight": null,
        "bg_color": "#2d2d3a",
        "text_color": "#ff6b6b",
        "border_color": "#556677",
        "border_width": 2,
        "border_type": "corners",
        "sensors": [
          {
            "name": "CPU",
            "description": "CPU usage percentage",
            "source": { "type": "perc_cpu" },
            "text_color": "#ffaa00",
            "bg_color": "#333333",
            "text_color_hover": "#ffffff",
            "bg_color_hover": "#444444",
            "border_color": "#ff6b6b",
            "border_color_hover": "#ffffff",
            "border_width": 2,
            "border_width_hover": 3,
            "border_radius": 8,
            "padding": 8
          }
        ]
      }
    ]
  }
}
```

`panel.font` is the family of **every** text the bar draws — sensor
names, values and descriptions, section titles, tray tooltips and the
notification popup; `panel.bold: true` renders all of it bold.

A sensor can declare `"icon"`: a freedesktop icon **name** (e.g. `"cpu"`)
drawn before its text, sized to the font and resolved under
`panel.icon_theme` (absent = the system icon-theme setting, then
`hicolor` — the same rule as the tray). Lookups are cached and the whole
icon machinery stays dormant — no theme parsing, no file resolution, no
memory — until at least one sensor declares an icon.

`panel.autostart: true` makes the bar launch, once at startup, the
applications marked as autostart under the standard XDG directories —
`$XDG_CONFIG_HOME/autostart` (default `~/.config/autostart`) and the
system `$XDG_CONFIG_DIRS/autostart` (default `/etc/xdg/autostart`) —
following the freedesktop Autostart spec: `Hidden`,
`X-GNOME-Autostart-enabled`, `OnlyShowIn`/`NotShowIn`, `TryExec`,
`Path`, `Exec` field codes, and a user file overriding the system file
with the same name. A program already running is never started twice,
and piuma never launches itself. Absent (the default) the feature is
dormant — nothing is scanned or spawned. The toggle lives in
piuma-setup's *Panel Settings* card.

`panel.app_theme: true` generates `<config dir>/theme/gtk.css` — a
small GTK stylesheet carrying the bar's **character** (`panel.font`,
`font_size`, `bold`), its **background/colours**
(`default_bg_color`, `default_text_color`) and its **borders**
(`sensor_defaults` / `section_defaults` border colour and radius) for
GTK applications. The file lives inside piuma's own directory, is
rewritten on config reload only when its content changes, and is
removed when the toggle goes off; piuma never edits your `gtk.css`,
`gtk-4.0/gtk.css` or `settings.ini`. Opt in once by adding
`@import url("file:///home/you/.config/piuma/theme/gtk.css");` at the
top of your `~/.config/gtk-3.0/gtk.css` (and `gtk-4.0/gtk.css`).

Coexistence: the bridge declares its own `@piuma_bg` / `@piuma_fg` /
`@piuma_border` names and joins your cascade additively, so an existing
customisation — a KDE/Breeze setup, or a hand-made palette like
`window_bg_color` — keeps winning wherever it sets the same property;
the bridge only fills the gaps. To let piuma win on a property, point
your own rule at it (`@define-color window_bg_color @piuma_bg;`).
Disabling the toggle removes only piuma's generated file. Qt
applications are not themed (that would require qt6ct/kvantum); they
keep their current style. The effect is best-effort — it follows each
app's CSS cascade.

Numeric style fields — `border_width`, `border_width_hover`, `border_radius`
and `padding` (on sensors, on sections, and on `panel.sensor_defaults` /
`panel.section_defaults`) use the range `-1..=N`:

- `-1` = **disabled**: the item does not override that field; the value
  comes from the panel defaults block (built-in fallback if absent there
  too);
- `0..=N` = **enabled**: a valid value that activates the padding / border /
  radius with that amount;
- a field missing from the config behaves like `-1`.

Sections support the same border drawing as sensors (`border_color`,
`border_width`, `border_radius`, `border_type` = `full`, `corners`,
`underline`, `upperline`, `left`, `right`; no hover variants), with the same
precedence rule as `panel.sensor_defaults`: a section owns the border group
only when it sets `border_color`, otherwise the whole group comes from
`panel.section_defaults`. `bg_color`/`text_color` are single keys: the
section value when set, otherwise the defaults, the panel
`default_bg_color`/`default_text_color`, then the built-in ones. A
`border_color` fully transparent (`#rrggbbaa` with alpha `00`) draws no
border at all: the border ring would otherwise punch a transparent
groove through the background.

`panel.section_defaults.respect_screen_edges: true` squares the section
corners that sit flush with a screen edge (roadmap M3): the bar's
anchored side always touches the screen, and on the length axis the
panel ends count as touching when `align` parks a section there
(`fill` = both ends, `left`/`top` = first section, `right`/`bottom` =
last one, `center` = none). Touching sides (corner included) render
square — background and border flush with the screen — only the sides
*away* from the edge keep the rounded corners.

### Sensor sources

Each sensor's `source` object has a `type` plus the parameters listed
(`—` = none). Output examples from a real run:

| `type` | output | params |
|---|---|---|
| `perc_cpu` | CPU usage (`4.8`) | — |
| `perc_gpu` | GPU utilization (`12`; NVIDIA or AMD) | — |
| `perc_mem` | RAM used percentage (`45%`) | — |
| `perc_swap` | swap used percentage (`0%`) | — |
| `perc_disk` | root fs used percentage (`66%`) | — |
| `perc_battery` | battery charge percentage (`80%`) | — |
| `perc_backlight` | backlight brightness percentage (`31%`) | — |
| `perc_volume` | mixer volume percentage (`50%`) | `device` (default `Master`; auto fallback `amixer` → `wpctl` → `pactl`) |
| `used_mem` | RAM used (`14353 MB`) | — |
| `used_swap` | swap used (`0 MB`) | — |
| `used_disk` | root fs used (`619G`) | — |
| `used_battery` | battery energy (`64.0 Wh`) | — |
| `used_netdown` | interface download rate (`   1.6 KB/s`) | `interface` |
| `used_netup` | interface upload rate (` 66.0 B/s`) | `interface` |
| `total_mem` | RAM total (`31807 MB`) | — |
| `total_swap` | swap total (`31806 MB`) | — |
| `total_disk` | root fs size (`950G`) | — |
| `total_battery` | full battery energy (`80.0 Wh`) | — |
| `detail_mem` | RAM used/total (`14353/31807 MB`) | — |
| `detail_swap` | swap used/total (`0/31806 MB`) | — |
| `detail_disk` | fs used/total (`619G/950G`) | — |
| `temp_cpu` | CPU package temperature (`65°C`, hwmon `coretemp`/`k10temp`) | — |
| `net_state` | interface state (`connected`) | `interface` |
| `uptime` | uptime (`up 2 hours, 30 minutes`) | — |
| `clock` | time (`10:12:42`); `format: "date"` = date only | `format` |
| `window` | focused window title (cut at 80 chars + `...`) | — |
| `workspace` | current workspace | — |
| `command` | shell command stdout | `command` |
| `env_var` | environment variable value | `env_var` |
| `static_text` | fixed text | `static_text` |
| `tray` | system tray icons (StatusNotifierItem) | — |
| `notifications` | newest active notification (`N6 newest (5)`) | — |

Network rates and the CPU percentage are computed from consecutive
counter samples (`/proc/net/dev`, `/proc/stat`): the very first refresh
can show `N/A` until the second sample arrives. The rate text has a
**fixed composition** (a 4-slot integer part padded with figure spaces
U+2007, one decimal digit, a space, the tier unit) and the sensor pins
its pixel width for the configured font, so a changing value or unit
never makes the sensor box — or its neighbours — resize (roadmap M5).

### System tray

A `tray` sensor renders the freedesktop StatusNotifierItem icons
(nm-applet, kdeconnect-indicator, …) on the bar; while no tray app
registers, the sensor renders nothing at all (no empty box):

```json
{ "name": "Tray", "source": { "type": "tray" } }
```

- **Watcher + host**: piuma speaks the `org.kde` interfaces (with the
  spec-literal `org.freedesktop.StatusNotifierItem` spelling as
  fallback), claims `org.kde.StatusNotifierWatcher` when it is free and
  always registers `org.kde.StatusNotifierHost-<pid>`, so items never
  fall back to the legacy XEmbed tray. When another tray already owns
  the watcher, piuma *observes* it instead — same item list, same
  registration signals — and takes over as soon as that watcher leaves.
- **Interaction**: left click → `Activate`, middle click →
  `SecondaryActivate`, right click → `ContextMenu` (coordinates are
  advisory hints in the spec), wheel → `Scroll` (120 units per notch),
  hover shows the item's tooltip (markup filtered out). Items in
  `NeedsAttention` swap to their attention icon and get a red frame;
  `NewIcon` / `NewStatus` / `NewToolTip` refresh the icon immediately,
  and a dead tray app removes its icon at once.
- **Sizing**: icons scale to the usable height of their sensor box —
  thickness minus section/sensor padding and borders (and the section
  title where sensors stack vertically) — clamped to 4–48px, so they
  fill the sensor and never overflow it.
- **Limitations (v1)**: the legacy XEmbed tray and `OverlayIcon*`
  overlays are not implemented; menus are not rendered host-side — the
  wheel/right-click are forwarded to the item instead (a `dbusmenu`
  host would be needed to draw them); in observer mode registrations
  that carry only an object path (no bus name) cannot be resolved and
  are skipped with a log line.

### Desktop notifications

With the `panel.notifications` block present, piuma claims
`org.freedesktop.Notifications` on the session bus (a freedesktop
notification server like mako/dunst) and pops the notifications up as a
card stack on the overlay layer, in the corner opposite the bar (or
wherever `popup_position` places it):

```json
"notifications": {
  "default_timeout_ms": 5000,
  "history_size": 20,
  "replace_existing": true,
  "popup_position": "auto",
  "popup_margin_h": 8,
  "popup_margin_v": 8,
  "font_size": 12,
  "text_color": "#ffffff", "text_color_hover": "#ffffff",
  "bg_color": "#000000", "bg_color_hover": "#000000",
  "border_color": "#808080", "border_color_hover": "#808080",
  "border_width": 1, "border_radius": 10, "border_type": "full",
  "padding": 12
}
```

The block's **presence** enables the whole feature (no block = server off,
no popup); its keys are optional and fall back to the values above:

- `default_timeout_ms` – fallback expiry when the client passes `-1`;
  `0` and urgency 2 (critical) never expire on their own.
- `history_size` – how many entries the wheel-browsable history keeps
  (the oldest are evicted when it is full; the popup only ever shows
  the newest 5 active ones).
- `replace_existing` – when an app sends an update for the same
  notification, show it on the **existing card** instead of adding a
  duplicate (same id, new content, fresh timer); `false` records every
  call as a new notification instead.
- `popup_position` – where the card stack appears (roadmap M4):
  `auto` (default) keeps the historic layout — the corner opposite the
  bar on the right; the explicit keys are `top-left`, `top-center`,
  `top-right`, `bottom-left`, `bottom-center`, `bottom-right`. Unknown
  values fall back to `auto`.
- `popup_margin_h` / `popup_margin_v` – horizontal and vertical margin
  from the screen edges in px (default `8`), applied to the two sides of
  an explicit `popup_position`.
- The style keys (`font_size`, `text_color[_hover]`, `bg_color[_hover]`,
  `border_color[_hover]`, `border_width`, `border_radius`, `border_type`,
  `padding`) dress the popup cards with the same fields as
  `panel.sensor_defaults` — the defaults are white text on black with a
  grey border, and an absent or `-1` numeric key means "use the
  default". The card height follows font and padding, so the surface
  stays click-exact for any style. All of it is editable in
  `piuma-setup` under *Desktop Notifications*.

```json
{ "name": "Notifiche", "source": { "type": "notifications" } }
```

The `notifications` sensor shows the newest active summary plus
`(selected/total)` while the wheel browses the history, and a left click
runs the shown entry's `default` action. On the popup, clicking a card
does the same (then dismisses the card) and the ✕ only dismisses. The
server emits the spec signals with the right reasons: 1 (expired),
2 (dismissed/closed by user), 3 (`CloseNotification`).

- **Limitations (v1)**: no sounds, no persistence across restarts, no
  do-not-disturb mode; body markup is stripped, actions other than
  `default` are not rendered (the spec's `activationToken` is ignored);
  icons come from `image-data`, `image-path` or the `app_icon` path
  (a themed icon *name* yields no icon).

### Portability

Sensor sources use the smallest practical set of tools:

- **Kernel interfaces only** (`/proc`, `/sys`) — work on any Linux
  distribution, including minimal/busybox systems: `perc_cpu`,
  `perc_mem`/`used_mem`/`total_mem`/`detail_mem`, the `perc_swap`/
  `used_swap`/`total_swap`/`detail_swap` families, `uptime`, `net_state`,
  `used_netdown`/`used_netup`, `temp_cpu` (Intel `coretemp`, AMD
  `k10temp`/`zenpower`), `perc_battery` (battery name is discovered, not
  hardcoded), `perc_backlight`.
- **Base userland** (coreutils — installed everywhere): `perc_disk` and
  the disk families (`df`), `clock` (`date`).
- **Optional tools with automatic fallbacks**: `perc_volume` tries
  `amixer` (ALSA) → `wpctl` (PipeWire) → `pactl` (PulseAudio) and sticks
  to the first that works — and so does the mouse wheel over the sensor,
  which raises/lowers the volume by 5% per notch (clamped 0–100%);
  `perc_gpu` tries
  `nvidia-smi` (NVIDIA) → the
  amdgpu sysfs counter (AMD), `N/A` on other GPUs.
- **Hyprland** (`hyprctl`): `window`, `workspace` — the only
  compositor-specific sensors, by design.
- **Yours**: `command` (any POSIX shell), `env_var`, `static_text`.

## Usage

1. Run with default config:
```bash
./target/release/piuma
```

2. Run with custom config:
```bash
./target/release/piuma --config /path/to/config.json
```

## Configuration GUI Tool

For a visual configuration tool, you can use:
1. The `piuma-setup` binary (if compiled)
2. Or use the command-line configuration wizard
3. Or manually edit the configuration file

`piuma-setup` loads `~/.config/piuma/piuma.json` on startup. The bottom bar
offers:

- **Load…** – edit any `*.json` file from `~/.config/piuma/` (different styles
  can live side by side);
- **Save** – writes the file being edited, asking for confirmation when there
  are unsaved changes;
- **Save As…** – saves under a new name, with an extra confirmation if the
  file already exists;
- **Reset** – restores the default configuration.

Each section and sensor header also shows **▲/▼** buttons that move it
up/down in the panel order (the arrow is disabled on the first/last item).

## Example Configurations

- `test_window_config.json` - Tests window title sensor
- `test_workspace_config.json` - Tests workspace sensor
- `test_config_full.json` - Tests all sensor types

## Building

```bash
cargo build --release
```

## License

Apache-2.0