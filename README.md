# Piuma - System Information Panel for Hyprland

Piuma is a lightweight system information panel for Hyprland/Wayland, designed as a replacement for Sway/Waybar status bar functionality.

## Features

### Sensor Types
- **System Sensors**: CPU, GPU, Memory, Disk, Uptime
- **Network Sensor**: Shows connection status of specified network interface
- **Audio Sensor**: Shows volume level of audio device
- **Clock Sensor**: Shows current time or date
- **Window Sensor**: Shows currently focused window title
- **Workspace Sensor**: Shows current workspace name

### Styling Features
- **Hover Effects**: Color changes on mouse hover
- **Border Styling**: Customizable borders with radius and width
- **Padding Control**: Space around sensor content
- **Section-level Styling**: Background and text colors per section

### Panel Positioning
- **Anchor Points**: top, bottom, left, right (supports combinations)
- **Alignment Options**: start, center, end
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
    "sections": [
      {
        "name": "System",
        "layout": "horizontal",
        "weight": null,
        "bg_color": "#2d2d3a",
        "text_color": "#ff6b6b",
        "sensors": [
          {
            "name": "CPU",
            "description": "CPU usage percentage",
            "source": { "type": "ProcCpu" },
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
and `padding` (on sensors and on `panel.sensor_defaults`) use the range
`-1..=N`:

- `-1` = **disabled**: the sensor does not override that field; the value
  comes from `panel.sensor_defaults` (built-in fallback if absent there too);
- `0..=N` = **enabled**: a valid value that activates the padding / border /
  radius with that amount;
- a field missing from the config behaves like `-1`.

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