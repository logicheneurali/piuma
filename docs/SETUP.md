# Piuma — the setup editor

[← Back to the README](../README.md)

`piuma-setup` is the graphical editor for `piuma.json`. It ships with the
release tarball and is built alongside the bar from source; if you prefer,
the same file can be edited by hand (see the
[configuration reference](CONFIGURATION.md)).

The running bar opens `piuma-setup` with a **right-click** anywhere it
has no widget of its own: tray icons and taskbar slots keep their own
right-click menus, every other zone starts the editor found next to the
bar binary.

`piuma-setup` loads `~/.config/piuma/piuma.json` on startup. The
categories sit behind **side tabs** on the left — Panel, Section
Defaults, Sensor Defaults, Sections, Icons, Wallpaper, Themes,
Notifications, Autostart apps, Layouts, Tweaks — and only the selected
category's card is shown, so the growing configuration stays readable;
unsaved edits live in memory, so switching tabs never loses work. The
bottom bar offers:

- **Restart Piuma** – restarts the bar (needed after adding or removing
  panels; saved edits otherwise hot-reload on their own);
- **Load…** – edit any `*.json` file from `~/.config/piuma/` (different styles
  can live side by side);
- **Save** – writes the file being edited;
- **Save As…** – saves under a new name, with an extra confirmation if the
  file already exists;
- **Reset** – restores the default configuration (undoable).

Keyboard: `Ctrl+S` save, `Ctrl+Shift+S` save as, `Ctrl+O` load,
`Ctrl+R` restart, `Ctrl+Z` / `Ctrl+Shift+Z` undo/redo, `Esc` closes a
dialog, `Enter` confirms the non-destructive ones. Closing the window or
loading another file with unsaved edits asks first (save / discard /
cancel), so an accidental click never drops work.

The *Section Defaults* tab also carries the two content paddings
(“Padding with horizontal layout” and “Padding with vertical layout”):
the inset the bar spends inside every section, around its title and
sensors, on a horizontal and a vertical bar respectively. They also
drive the automatic icon size, so the preview always shows the same fit
the bar will draw.

Each section and sensor header also shows **▲/▼** buttons that move it
up/down in the panel order (the arrow is disabled on the first/last item),
a **Duplicate** button clones a section (sensors included) right below it.
The Sections tab has a filter over section titles and the sensors' name,
description or source type. Every colour field carries the current
palette's tones as one-click chips, so matching a colour never means
retyping a hex value.
