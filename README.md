# Piuma

A lightweight system panel (status bar) for Hyprland/Wayland — an
alternative to Sway/Waybar. It shows system sensors, a system tray,
desktop notifications and your workspaces, and it is meant to be
**easy to set up**: you configure it with a graphical tool, not by
hand-editing JSON.

---

## The short version

```bash
git clone https://github.com/your-repo/piuma.git
cd piuma
cargo build --release
./target/release/piuma          # the bar
./target/release/piuma-setup    # the graphical configurator
```

1. Run `piuma-setup`.
2. Pick a section, add a sensor, choose what it shows (CPU, clock,
   volume…), pick colours if you want.
3. Press **Save** — the bar picks it up.

You never *have* to touch the JSON by hand. The file lives at
`~/.config/piuma/piuma.json` and is only there if you want fine control
or want to keep several looks side by side.

---

## Installation

**Requirements:** a Rust toolchain (`cargo`) and Hyprland. Build with:

```bash
cargo build --release
```

This produces two programs in `target/release/`:

| binary | what it does |
|---|---|
| `piuma` | the bar itself |
| `piuma-setup` | the graphical configuration editor |

Run the bar:

```bash
./target/release/piuma                        # default config
./target/release/piuma --config /path/to.json # a specific config file
```

Only one bar runs at a time: launching `piuma` again replaces the
previous instance (coordinated through a lock file in
`$XDG_RUNTIME_DIR/piuma.lock`), so starting it from your compositor
config after a restart never stacks a second bar. The bar also exits
for good when the display connection ends (compositor closed, session
over) instead of lingering in the background.

To start it automatically with Hyprland, add it to your `hyprland.lua`:

```lua
hl.on("hyprland.start", function()
  hl.exec_cmd("/path/to/piuma/target/release/piuma")
end)
```

---

## Configuration with `piuma-setup`

`piuma-setup` loads `~/.config/piuma/piuma.json` on startup and shows
one category at a time through the tabs on the left:

- **Panel** – fonts, colours, position, alignment
- **Sections** – the groups on the bar, their order and layout
- **Sensor Defaults** / **Section Defaults** – the look inherited by
  every sensor / section
- **Desktop Notifications** – the notification server and its popups
- **Wallpaper** – the built-in desktop background
- **Autostart apps** – which programs launch at startup

Unsaved edits live in memory, so switching tabs never loses work. The
bottom bar holds the file actions:

- **Load…** – edit any `*.json` from `~/.config/piuma/` (different
  styles can live side by side)
- **Save** – write the file being edited
- **Save As…** – save under a new name
- **Reset** – back to the default configuration

Every section and sensor header also has **▲/▼** buttons to move it up
or down in the bar's order.

Sensors come from the **Sections** tab: add one, give it a *source*
type (see the table below) and that is a working sensor — colours,
borders and padding all have sensible defaults and are optional.

---

## The configuration file

Only if you prefer editing by hand: the config is a single JSON file at
`$XDG_CONFIG_HOME/piuma/piuma.json` (default
`~/.config/piuma/piuma.json`). A minimal example with one section and
two sensors:

```json
{
  "panel": {
    "font_size": 12,
    "anchor": "bottom",
    "align": "center",
    "sections": [
      {
        "name": "System",
        "sensors": [
          { "name": "CPU", "source": { "type": "perc_cpu" } },
          { "name": "Time", "source": { "type": "clock" } }
        ]
      }
    ]
  }
}
```

### Sensor types

Each sensor's `source` has a `type` plus the parameters listed
(`—` = none). Output shown is from a real run.

| `type` | output | params |
|---|---|---|
| `perc_cpu` | CPU usage (`4.8`) | — |
| `perc_gpu` | GPU utilization (`12`; NVIDIA or AMD) | — |
| `perc_mem` | RAM used percentage (`45`) | — |
| `perc_swap` | swap used percentage (`0`) | — |
| `perc_disk` | root fs used percentage (`66`) | — |
| `perc_battery` | battery charge percentage (`80`) | — |
| `perc_backlight` | backlight brightness percentage (`31`) | — |
| `perc_volume` | mixer volume (`50`) | `device` (default `Master`; auto fallback `amixer` → `wpctl` → `pactl`) |
| `used_mem` | RAM used (`14353 MB`) | — |
| `used_swap` | swap used (`0 MB`) | — |
| `used_disk` | root fs used (`619G`) | — |
| `used_battery` | battery energy (`64.0 Wh`) | — |
| `used_netdown` | interface download rate (`1.6 KB/s`) | `interface` |
| `used_netup` | interface upload rate (`66.0 B/s`) | `interface` |
| `total_mem` | RAM total (`31807 MB`) | — |
| `total_swap` | swap total (`31806 MB`) | — |
| `total_disk` | root fs size (`950G`) | — |
| `total_battery` | full battery energy (`80.0 Wh`) | — |
| `detail_mem` | RAM used/total (`14353/31807 MB`) | — |
| `detail_swap` | swap used/total (`0/31806 MB`) | — |
| `detail_disk` | fs used/total (`619G/950G`) | — |
| `temp_cpu` | CPU temperature (`65°C`, hwmon `coretemp`/`k10temp`) | — |
| `net_state` | interface state (`connected`) | `interface` |
| `uptime` | uptime (`up 2 hours, 30 minutes`) | — |
| `clock` | time (`10:12:42`); `format: "date"` = date only | `format` |
| `window` | focused window title (cut at 80 chars) | — |
| `workspace` | one clickable button per workspace | — |
| `command` | shell command stdout | `command` |
| `env_var` | environment variable value | `env_var` |
| `static_text` | fixed text | `static_text` |
| `tray` | system tray icons (StatusNotifierItem) | — |
| `notifications` | newest active notification | — |

Type names are prefixed by category: `perc_` percentage, `used_`
consumption, `total_` full value, `detail_` used/total, `temp_`
temperature. The old names (`ProcCpu`, `Audio`, `Clock`, …) still load,
and `piuma-setup` converts them on save.

Network rates and the CPU percentage are computed from consecutive
samples, so the very first refresh can briefly show `N/A`. Their text
keeps a fixed pixel width, so a changing value never makes the bar
jump around.

### What each sensor needs

Most sensors work on any Linux system; a few need extra tools:

- **Kernel only** (`/proc`, `/sys`): cpu, mem, swap, uptime, network,
  temperature, battery, backlight
- **coreutils** (installed everywhere): disk (`df`), clock (`date`)
- **Optional, with automatic fallback**: volume (`amixer` → `wpctl` →
  `pactl`; the mouse wheel over the sensor adjusts the volume), GPU
  (`nvidia-smi` or AMD sysfs)
- **Hyprland** (`hyprctl`): `window`, `workspace`
- **Yours**: `command`, `env_var`, `static_text`

---

## Styling

Everything has a default; every styling key is optional.

- **Colours** on sensors, sections and the panel itself, including
  separate **hover** colours (`bg_color_hover`, `text_color_hover`,
  `border_color_hover`).
- **Borders**: colour, width, radius and style (`full`, `corners`,
  `underline`, `upperline`, `left`, `right`). Sections support the same
  border drawing as sensors.
- **Padding** around sensor content.
- **Position**: `anchor` at `top`/`bottom`/`left`/`right` (left/right
  give a vertical bar); `align` as `left`/`center`/`right` or `fill`
  to spread the sections across the whole bar with equal spacers;
  `percentage` sets how much of the screen edge the bar covers.
- Numeric style fields (`border_width`, `border_radius`, `padding`)
  accept `-1` for "disabled — inherit the default", `0..=N` to enable
  with that amount.
- `panel.section_defaults.respect_screen_edges: true` squares the
  corners that sit flush with a screen edge, keeping rounding only on
  the sides away from it.

### Sensor extras

- A sensor can declare `"icon"`: a freedesktop icon **name** (e.g.
  `"cpu"`) drawn before its text, resolved under `panel.icon_theme`
  (absent = the system setting, then `hicolor`) — the same rule as the
  tray.
- A sensor can declare `"draw_graph": true`: hovering it shows the
  value's trend over the last two minutes in a popup at the cursor.
  Works for the numeric sources (`perc_*`, `used_*`, `temp_cpu`).
  Toggleable in `piuma-setup` (*Draw graph*).

---

## System tray

A `tray` sensor shows the freedesktop StatusNotifierItem icons
(nm-applet, KDE Connect, …); while no tray app registers, the sensor
renders nothing at all (no empty box):

```json
{ "name": "Tray", "source": { "type": "tray" } }
```

- Left/middle/right click follow the item's own methods (`Activate`,
  `SecondaryActivate`, menus), the wheel scrolls, hovering shows the
  item's tooltip; items asking for attention get their attention icon
  and a red frame.
- Menus are piuma's own window next to the icon: submenus stay
  expanded, toggles carry ✓, disabled entries are dimmed.
- Icons size themselves to the usable height of their box (clamped
  4–48px), tray and taskbar icons always match each other, and a row
  that does not fit wraps onto a new line instead of overflowing.
- Not implemented (v1): the legacy XEmbed tray, per-row menu icons,
  keyboard navigation.

---

## Desktop notifications

With the `panel.notifications` block present, piuma becomes a
freedesktop notification server (like mako/dunst) and pops cards up in
the corner opposite the bar:

```json
"notifications": {
  "default_timeout_ms": 5000,
  "history_size": 20,
  "replace_existing": true,
  "popup_position": "auto",
  "font_size": 12,
  "text_color": "#ffffff",
  "bg_color": "#000000",
  "border_color": "#808080",
  "border_width": 1,
  "border_radius": 10,
  "padding": 12
}
```

The block's **presence** enables everything (no block = no server, no
popup); its keys are optional and the values above are the defaults.
Highlights: up to 5 cards at once, click a card to run its `default`
action, wheel over a `notifications` sensor to browse history,
critical notifications never expire, `popup_position` accepts
`top-left` … `bottom-right` (or `auto`). All of it is editable in
`piuma-setup` under *Desktop Notifications*.

- Not implemented (v1): sounds, persistence across restarts,
  do-not-disturb, body markup, non-`default` actions.

---

## Optional extras

- **`panel.autostart: true`** – launch your autostart applications
  once at startup, following the freedesktop Autostart spec
  (`~/.config/autostart`, `/etc/xdg/autostart`). A program already
  running is never started twice. Manage it per-app in `piuma-setup`
  (*Autostart apps*).
- **`panel.app_theme: true`** – generate
  `~/.config/piuma/theme/gtk.css` carrying the bar's font, colours and
  borders, so GTK apps match the bar. Opt in once by adding
  `@import url("file:///home/you/.config/piuma/theme/gtk.css");` to
  `~/.config/gtk-3.0/gtk.css` (and `gtk-4.0/gtk.css`). Your own rules
  keep winning wherever they set the same property; Qt apps are not
  themed.
- **`panel.wallpaper`** – let the bar draw the desktop background
  itself, no external wallpaper tool needed:
  `"wallpaper": { "enabled": true, "path": "/path/to/image.png" }`.
  One background surface per monitor, image shown with a cover fit,
  input passes through to the desktop. The toggle plus a browser of the
  images found in the standard folders live in `piuma-setup`
  (*Wallpaper*).

---

## Example configurations

- `test_window_config.json` – window title sensor
- `test_workspace_config.json` – workspace sensor
- `test_config_full.json` – all sensor types

## License

Apache-2.0
