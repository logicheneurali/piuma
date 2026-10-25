# Piuma - System Information Panel for Hyprland

Piuma is a lightweight system information panel for Hyprland/Wayland.

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
  `workspace`, `command`, `env_var`, `static_text`, `tray`, `notifications`,
  `taskbar`, `layout`

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
- **Rounded Screen Corners**: `panel.screen_corners { enable, color, radius }`
  masks each display corner with a solid-colour square minus the inner
  quarter disk, so the screen itself reads as rounded like the panel —
  drawn on layer-shell overlay surfaces, so it needs a layer-shell
  compositor (Hyprland: works)

### Panel Positioning

- **Anchor Points**: top, bottom, left, right (supports combinations)
- **Alignment Options**: left, center, right (top/bottom on vertical panels)
- **Fill Mode**: `align: "fill"` spreads the sections across the whole panel
  with equal spacers, so the panel always spans 100% of the screen; with
  exactly 3 sections the middle one is kept centered when there is enough
  room (otherwise it falls back to the equal-spacer distribution)
- **Percentage-based filling**: Configure percentage of available space

## Requirements

To build and run Piuma you need:

- **Hyprland on Wayland** — the `window` and `workspace` sensors talk to
  the compositor through `hyprctl`; every other sensor is
  compositor-independent
- **A Rust toolchain** (`cargo`) to build from source — if you don't have
  one, install it from [rustup.rs](https://rustup.rs)
- **A few common command-line tools**, depending on which sensors you
  enable: `df` and `date` (coreutils), a mixer tool for volume
  (`amixer`, `wpctl` or `pactl`) and `nvidia-smi` or AMD sysfs for the
  GPU. All remaining sensors read `/proc` and `/sys` directly and need
  nothing extra — see [Portability](#portability) for the full mapping.

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

`panel.app_theme: true` generates a complete GTK theme at
`~/.local/share/themes/Piuma`: piuma copies the Adwaita stylesheets of
`adw-gtk3-dark` (the widget rules, for GTK3 and GTK4) and appends its
own palette block, so the theme is self-contained and follows the
Adwaita schema. The block maps the bar's three layers: **windows**
(window, headerbar, sidebar, dialog) use the panel palette,
**containers** (views, lists, cards) the section palette, **interactive
widgets** (buttons, entries, combos, menus) the sensor palette; every
hover uses that layer's hover tone. Translucent tones are flattened
onto the surface behind them so nested GTK widgets cannot blend twice.

GTK loads `~/.config/gtk-{3,4}.0/gtk.css` at USER priority, above any
theme, so piuma also appends the same block to those two files between
`piuma theme begin` / `piuma theme end` markers — otherwise an existing
customisation would keep pinning the old colours. Disabling the toggle
removes the marked block and the theme directory, leaving the rest of
your files untouched.

When `app_theme` is on, piuma injects `GTK_THEME=Piuma`,
`QT_QPA_PLATFORMTHEME=kde` and `QT_STYLE_OVERRIDE=Fusion` into
Hyprland's environment via a marked block in
`~/.config/hypr/hyprland.lua`, followed by a compositor
`reload config-only`, so new applications pick them up automatically.
Qt apps take their palette from the platform theme: the KDE plugin is
the one that reads a configurable palette, and the Fusion style is
needed because Breeze draws its own colours. piuma therefore patches
the `[Colors:*]` groups of `~/.config/kdeglobals` (windows → panel,
views → section, buttons/selection → sensor, disabled text dimmed) and
writes `~/.local/share/color-schemes/Piuma.colors`; the untouched
`kdeglobals` is kept at `<config dir>/theme/kdeglobals.pre-piuma` and
put back, with the scheme file removed, when the toggle goes off.
Applications already running keep their palette until restarted. Qt5
apps fall back to their own style (no KDE platform-theme plugin for
Qt5 is installed); Qt6 apps, including the KDE ones, follow the
palette.

Session exit: logind and systemd stop a session with SIGTERM, which
piuma catches to restore `kdeglobals`, remove the marked block from
`gtk.css`, remove the Hyprland env block and delete the theme and
scheme files. A `loginctl` logout and a normal `reboot`/`poweroff` are
therefore clean: a Plasma or GNOME session started afterwards sees none
of piuma's colours. A forced reboot, a crash or `kill -9` cannot be
caught, but the next piuma start with the toggle off restores the files
anyway. The environment of the running Hyprland session cannot be
un-set without restarting the compositor, so a GTK app launched there
after piuma stopped falls back to the default theme (with a warning).

Other applications. Two mechanisms, both reverted on exit:

- **Environment-selected config dirs** — piuma writes a directory under
  `<config dir>/theme/` and points the app at it, leaving the user's
  files untouched: `KITTY_CONFIG_DIRECTORY` (our `kitty.conf` includes
  yours, then sets colours), `MPV_HOME` (`mpv.conf` includes yours,
  `script-opts/osc.conf` carries the OSC palette; `scripts`, `fonts`,
  `input.conf` and `watch_later` are symlinked), `YAZI_CONFIG_HOME`
  (`theme.toml` generated, your `yazi.toml`/`keymap.toml`/`vfs.toml`
  symlinked).
- **Marked includes in your files** — reverted from the same markers:
  foot (`[colors-dark]` block; foot 1.28 has no top-level include at
  the end), rofi (rules inline in `config.rasi`: a second `@theme` would drop the user theme's structure; font, size (floored at 12px) and weight come from the panel, border width/radius from piuma (collapsed to 0 when the configured border colour is transparent, as the bar has no hover border), and the selected row uses the hover tone), btop (`color_theme = "piuma"` with a
  generated theme file and the previous name restored), alacritty (the
  palette file is added to `[general] import`; the original line is
  kept and put back).

Background, foreground, selection, cursor and accent come from the
three layers; ANSI colours and syntax highlighting are deliberately
left alone so command output and code stay readable.

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

| `type`           | output                                                       | params                                                                  |
| ---------------- | ------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `perc_cpu`       | CPU usage (`4.8`)                                            | —                                                                       |
| `perc_gpu`       | GPU utilization (`12`; NVIDIA or AMD)                        | —                                                                       |
| `perc_mem`       | RAM used percentage (`45%`)                                  | —                                                                       |
| `perc_swap`      | swap used percentage (`0%`)                                  | —                                                                       |
| `perc_disk`      | root fs used percentage (`66%`)                              | —                                                                       |
| `perc_battery`   | battery charge percentage (`80%`)                            | —                                                                       |
| `perc_backlight` | backlight brightness percentage (`31%`)                      | —                                                                       |
| `perc_volume`    | mixer volume percentage (`50%`)                              | `device` (default `Master`; auto fallback `amixer` → `wpctl` → `pactl`) |
| `used_mem`       | RAM used (`14353 MB`)                                        | —                                                                       |
| `used_swap`      | swap used (`0 MB`)                                           | —                                                                       |
| `used_disk`      | root fs used (`619G`)                                        | —                                                                       |
| `used_battery`   | battery energy (`64.0 Wh`)                                   | —                                                                       |
| `used_netdown`   | interface download rate (`   1.6 KB/s`)                      | `interface`                                                             |
| `used_netup`     | interface upload rate (` 66.0 B/s`)                          | `interface`                                                             |
| `total_mem`      | RAM total (`31807 MB`)                                       | —                                                                       |
| `total_swap`     | swap total (`31806 MB`)                                      | —                                                                       |
| `total_disk`     | root fs size (`950G`)                                        | —                                                                       |
| `total_battery`  | full battery energy (`80.0 Wh`)                              | —                                                                       |
| `detail_mem`     | RAM used/total (`14353/31807 MB`)                            | —                                                                       |
| `detail_swap`    | swap used/total (`0/31806 MB`)                               | —                                                                       |
| `detail_disk`    | fs used/total (`619G/950G`)                                  | —                                                                       |
| `temp_cpu`       | CPU package temperature (`65°C`, hwmon `coretemp`/`k10temp`) | —                                                                       |
| `net_state`      | interface state (`connected`)                                | `interface`                                                             |
| `uptime`         | uptime (`up 2 hours, 30 minutes`)                            | —                                                                       |
| `clock`          | time (`10:12:42`); `format: "date"` = date only              | `format`                                                                |
| `window`         | focused window title (cut at 80 chars + `...`)               | —                                                                       |
| `workspace`      | current workspace                                            | —                                                                       |
| `command`        | shell command stdout                                         | `command`                                                               |
| `env_var`        | environment variable value                                   | `env_var`                                                               |
| `static_text`    | fixed text                                                   | `static_text`                                                           |
| `tray`           | system tray icons (StatusNotifierItem)                       | —                                                                       |
| `notifications`  | newest active notification (`N6 newest (5)`)                 | —                                                                       |

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
  stays click-exact for any style, and the card's text scrolls inside
  it (wheel) when a long body — or one with line breaks — does not
  fit. All of it is editable in `piuma-setup` under *Desktop
  Notifications*.

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

### Window layouts

A `layout` sensor saves and re-applies the arrangement of the windows
in the current workspace. Everything lives in one file,
`~/.config/piuma/piuma-layouts.json` (next to `piuma.json`): **while
the file is absent the layout system is ignored** — the sensor shows
no value at all — and the **first click creates it** with the current
arrangement.

A **click saves** the visible windows of the workspace you are on.
Layouts are **named automatically** from their (class, workspace) pairs
sorted alphabetically — titles are left out because a terminal follows
its directory and a browser its tab. The same window set always
produces the same name, so saving again replaces that layout instead of
duplicating it; a re-save also folds the title-keyed layouts written by
older versions into the new entry.

The bar **re-applies a saved layout automatically** when a window is
**created, closed or moved to another workspace**: the activation check
runs on those events (never on a timer). Matching is by **class**, so a
changed title never blocks a layout. The most specific saved layout
whose classes are all open fires once for that window set; windows of
the same class are paired by exact title, then by the window already on
the layout's workspace, then by title resemblance, then by position —
so an app with several identical windows still restores. A workspace
move only re-evaluates the match: a layout already applied for the
current window set is **not** applied again, so dragging a window
between workspaces is never undone. An extra window on the layout's own
workspace keeps it unapplied (other workspaces never veto). Moving
windows by hand in between is never fought.

```json
{ "name": "Layout", "source": { "type": "layout" } }
```

(A legacy `"name"` key in the source is ignored: layouts are named by
their window set.)

- **State on the bar**: `⋯` armed and waiting, `✓` applied,
  `✓ saved` right after the click, no value while the file is absent.
- **What is restored**: the workspace of each window and, for a
  floating one, the exact pixel position and size; a tiled one goes
  back into the tiler (and there only — never floated) with its saved
  **size**, applied *measured*: an 8 px probe teaches each edge which
  way it really answers (a bottom window grows by asking up), the
  exact ask follows from that sign, and anything that would push a
  tile under 80 px — or fails to shrink the gap — reverts at once. An
  edge the probe cannot move (a full-width tile's x is the monitor's
  own border) or a tree that cannot hold the saved shape at all (a
  kill + reopen re-inserts the window elsewhere) is left to the
  tiler: no window is ever floated or strangled to force it. A window
  already at the saved size is left untouched. Fullscreen windows get
  no geometry either.
- **Matching**: by class *and* window title, with the exact saved
  count — no saved window missing, no extra twin open — and no
  unrelated window on the layout's own workspace either: the layout
  describes that workspace's window set exclusively, so an extra
  window there keeps it unapplied. Windows on other workspaces never
  block it. When **no** saved title fits anymore (a terminal followed
  its directory, a browser its tab), a fallback picks the layout with
  the very same window classes — the same count per class on its own
  workspaces, so the extra-window veto holds by class too — whose
  titles still **resemble** the open ones the most, and only when
  that winner is clear: a tie applies nothing rather than guessing
  between two same-class layouts.
- **Managing layouts**: piuma-setup's **Layouts** tab lists them
  (their windows as `class: title`) and deletes a single one — or all
  of them, which removes the file and turns the system off — both
  behind a confirmation.
- **Limitations**: every `layout` sensor shares the one file and shows
  the same state; layout files edited by hand — or deletions made in
  the setup — reach the running bar on its next restart; monitor
  placement is derived from the saved pixel position only.

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

`piuma-setup` loads `~/.config/piuma/piuma.json` on startup. The
categories sit behind **side tabs** on the left — Panel, Notifications,
Sensor Defaults, Section Defaults, Sections — and only the selected
category's card is shown, so the growing configuration stays readable;
unsaved edits live in memory, so switching tabs never loses work. The
bottom bar offers:

- **Load…** – edit any `*.json` file from `~/.config/piuma/` (different styles
  can live side by side);
- **Save** – writes the file being edited, asking for confirmation when there
  are unsaved changes;
- **Save As…** – saves under a new name, with an extra confirmation if the
  file already exists;
- **Reset** – restores the default configuration.

Each section and sensor header also shows **▲/▼** buttons that move it
up/down in the panel order (the arrow is disabled on the first/last item).

## License

Apache-2.0