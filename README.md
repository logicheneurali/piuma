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
  `workspace`, `command`, `env_var`, `static_text`

The old names (`ProcCpu`, `NvidiaGpu`, `ProcMem`, `ProcDisk`, `Audio`,
`Network`, `Uptime`, `Clock`, `Window`, `Workspace`, `Command`, `EnvVar`,
`StaticText`) are still accepted, so existing configs keep loading;
`piuma-setup` converts them to the canonical names on load, so the next
save writes the new names into `piuma.json`.

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
`default_bg_color`/`default_text_color`, then the built-in ones.

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
| `used_netdown` | interface download rate (`1.6 KB/s`) | `interface` |
| `used_netup` | interface upload rate (`66 B/s`) | `interface` |
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
| `window` | focused window title | — |
| `workspace` | current workspace | — |
| `command` | shell command stdout | `command` |
| `env_var` | environment variable value | `env_var` |
| `static_text` | fixed text | `static_text` |

Network rates and the CPU percentage are computed from consecutive
counter samples (`/proc/net/dev`, `/proc/stat`): the very first refresh
can show `N/A` until the second sample arrives.

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