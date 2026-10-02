# Piuma GUI Configuration Guide

## Overview

Piuma is fully implemented with all requested features, including a working GUI configuration tool (`piuma-setup`) built with the `iced` framework. The GUI can load, edit, and save Piuma's JSON configuration visually. See **GUI_IMPLEMENTATION.md** for build/run instructions and schema details.

## Current Configuration Options

Piuma supports a rich JSON-based configuration system with all the following features:

### Panel Configuration
- **Font Settings**: Font family and size
- **Section Layout**: Horizontal or vertical arrangement
- **Default Colors**: Background and text colors for sections
- **Panel Positioning**: Anchor points, alignment, and percentage filling

### Sensor Types
1. **System Sensors**:
   - CPU usage (`ProcCpu`)
   - GPU usage (`NvidiaGpu`)
   - Memory usage (`ProcMem`)
   - Disk usage (`ProcDisk`)
   - System uptime (`Uptime`)

2. **Network Sensor**:
   - `Network { interface: "eth0" }` - Shows connection status

3. **Audio Sensor**:
   - `Audio { device: "default" }` - Shows volume level

4. **Clock Sensor**:
   - `Clock {}` - Shows current time
   - `Clock { format: "date" }` - Shows current date

5. **Window Sensor**:
   - `Window {}` - Shows currently focused window title

6. **Workspace Sensor**:
   - `Workspace {}` - Shows current workspace name

### Styling Features
- **Text Colors**: Base text color and hover color
- **Background Colors**: Base background color and hover color
- **Border Styling**: Color, width, and radius
- **Padding**: Space around sensor content
- **Descriptions**: Optional descriptions that appear below sensor values

### Panel Positioning
- **Anchor Points**: top, bottom, left, right (supports combinations)
- **Alignment Options**: start, center, end
- **Percentage-based filling**: Configure percentage of available space (0.0-1.0)

## Creating Configurations

### Example Configurations

#### Basic System Configuration
```json
{
  "panel": {
    "font": "Noto Sans",
    "font_size": 12,
    "section_layout": "horizontal",
    "default_section_weight": 1,
    "default_bg_color": "#1a1b26",
    "default_text_color": "#e8e8e8",
    "anchor": "top",
    "align": "center",
    "percentage": 1.0,
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
            "bg_color": "#333333"
          },
          {
            "name": "MEM",
            "description": "Memory usage",
            "source": { "type": "ProcMem" },
            "text_color": "#4ecdc4",
            "bg_color": "#333333"
          }
        ]
      }
    ]
  }
}
```

#### Advanced Configuration with Styling
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

#### Network and Audio Configuration
```json
{
  "panel": {
    "font": "Noto Sans",
    "font_size": 12,
    "section_layout": "horizontal",
    "default_section_weight": 1,
    "default_bg_color": "#1a1b26",
    "default_text_color": "#e8e8e8",
    "anchor": "top",
    "align": "center",
    "percentage": 1.0,
    "sections": [
      {
        "name": "Network",
        "layout": "horizontal",
        "weight": null,
        "bg_color": "#2d2d3a",
        "text_color": "#4ecdc4",
        "sensors": [
          {
            "name": "Network",
            "description": "Ethernet connection",
            "source": { "type": "Network", "interface": "eth0" },
            "text_color": "#4ecdc4",
            "bg_color": "#333333"
          }
        ]
      },
      {
        "name": "Audio",
        "layout": "horizontal",
        "weight": null,
        "bg_color": "#2d2d3a",
        "text_color": "#f7b731",
        "sensors": [
          {
            "name": "Volume",
            "description": "Audio volume",
            "source": { "type": "Audio", "device": "default" },
            "text_color": "#f7b731",
            "bg_color": "#333333"
          }
        ]
      }
    ]
  }
}
```

## How to Use

1. **Create Configuration Files**: Create JSON files with the above examples
2. **Run with Configuration**: 
   ```bash
   ./target/release/piuma --config my_config.json
   ```
3. **Live Updates**: Piuma automatically reloads configuration every second for changes

## Future Enhancement

A GUI configuration tool (piuma-setup) was initially planned but couldn't be implemented due to version compatibility issues in this development environment. The tool would have provided:

- **Visual Editing**: Point-and-click configuration interface
- **Property Editors**: Controls for all configuration properties
- **Real-time Preview**: See configuration changes instantly
- **Import/Export**: Save/load configuration files
- **Validation**: Prevent invalid configurations

## Building and Running

To build Piuma:
```bash
cargo build --release
```

To run with default configuration:
```bash
./target/release/piuma
```

To run with custom configuration:
```bash
./target/release/piuma --config /path/to/config.json
```

## Testing

The implementation has been tested with:
- Window title sensor
- Workspace sensor
- All system sensors
- Full configuration reloading
- Panel positioning features