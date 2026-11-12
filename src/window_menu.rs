//! The floating window-actions menu (`panel.tweaks.window_menu`).
//!
//! One small Overlay layer surface follows the focused window with
//! close / float / sticky buttons. A socket thread listens to
//! Hyprland's event socket and every relevant event reads
//! `j/activewindow` + `j/monitors` on the persistent request socket
//! (no fork, ~0.3 ms) to learn the window's rect, output and flags.
//! The surface is created once and moved with `MarginChange`; a focus
//! change glides it over `ANIM` — the only frames this module asks for.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::widget::{container, image, svg, text, MouseArea, Row};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Task};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, Margin, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;

use crate::config::{self, WindowMenuPosition};
use crate::palette;
use crate::styles::{default_text, panel_background};
use crate::windows;
use crate::{icons, Message, Panel};


/// Icon edge in px: the size the bar's own rows render at.
const ICON: u32 = 16;
/// Space between two buttons inside the pill.
const GAP: u32 = 4;
/// Distance between the window edge and the menu.
const MARGIN: i32 = 4;
/// Glide time of a focus change: fast enough to feel like a move.
const ANIM: Duration = Duration::from_millis(120);
/// One animation frame (60 fps, only while a glide or a fade is alive).
pub(crate) const FRAME: Duration = Duration::from_millis(16);
/// Idle fade-out pace: 10% of the opacity per second.
const FADE_OUT: Duration = Duration::from_secs(10);
/// Input fade-in: back to full opacity in two seconds.
const FADE_IN: Duration = Duration::from_secs(2);

/// The three window actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    Close,
    Float,
    Sticky,
}

impl Action {
    pub(crate) const ALL: [Action; 3] = [Action::Close, Action::Float, Action::Sticky];

    /// The theme icon, the fallback letter and the tooltip-free label.
    fn icon(self, pinned: bool) -> &'static str {
        match self {
            Action::Close => "window-close",
            Action::Float => "window-duplicate",
            Action::Sticky if pinned => "window-unpin",
            Action::Sticky => "window-pin",
        }
    }

    fn letter(self) -> &'static str {
        match self {
            Action::Close => "X",
            Action::Float => "F",
            Action::Sticky => "S",
        }
    }

    /// The taskbar menu's row label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Action::Close => "Close",
            Action::Float => "Float",
            Action::Sticky => "Sticky",
        }
    }
}

/// The focused window as the menu needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) address: String,
    /// Output-local rect `(x, y, w, h)`.
    pub(crate) rect: (i32, i32, i32, i32),
    /// The output that holds the window: its size picks the clamping bounds.
    pub(crate) screen: (u32, u32),
    /// A layer surface cannot move between outputs: this pins the surface.
    pub(crate) output: String,
    pub(crate) floating: bool,
    pub(crate) pinned: bool,
    pub(crate) fullscreen: bool,
}

/// A linear opacity run: the idle fade-out or the input fade-in.
#[derive(Clone, Copy)]
struct Fade {
    from: f32,
    to: f32,
    started: Instant,
    duration: Duration,
}

impl Fade {
    fn alpha(&self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32())
        .clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) >= self.duration
    }
}

/// The live menu surface and what it currently shows.
pub(crate) struct WindowMenu {
    pub(crate) id: iced::window::Id,
    /// The window it follows; `None` = nothing is focused.
    pub(crate) target: Option<Target>,
    pub(crate) hover: Option<Action>,
    /// A glide in progress: where it started and when.
    pub(crate) from: Option<((i32, i32), Instant)>,
    /// Resting opacity (`1` = fully visible): the fade's endpoint.
    alpha: f32,
    /// The idle/input fade in progress, if any.
    fade: Option<Fade>,
    /// The surface's output: a different one needs a new surface.
    pub(crate) output: Option<String>,
    /// Last applied surface top-left, so a still window sends nothing.
    pub(crate) at: (i32, i32),
    /// The pill's size (the surface may be `MARGIN` taller, see `bridge`).
    pub(crate) size: (u32, u32),
    /// Strip side; changing it changes the surface height, so it needs a
    /// new surface.
    pub(crate) bridge: Option<Bridge>,
}

impl WindowMenu {
    /// Whether a glide or a fade still needs animation frames.
    pub(crate) fn animating(&self) -> bool {
        self.from.is_some() || self.fade.is_some()
    }

    /// The opacity to render at: the running fade's value, else the
    /// resting one.
    pub(crate) fn alpha(&self) -> f32 {
        self.fade.as_ref().map_or(self.alpha, |f| f.alpha(Instant::now()))
    }
}

/// The pill's size in px: three icon buttons in a row.
pub(crate) fn menu_size(padding: u16) -> (u32, u32) {
    (
        3 * ICON + 2 * GAP + 2 * u32::from(padding),
        ICON + 2 * u32::from(padding),
    )
}

/// The resolved look of the default sensor: the menu borrows it whole.
struct MenuStyle {
    bg: Color,
    bg_hover: Color,
    text: Color,
    border: Color,
    border_width: u16,
    border_radius: u16,
    border_type: String,
    padding: u16,
}

/// The default sensor's style with the wallpaper palette's precedence;
/// a config without sensor defaults falls back to the panel's tones so
/// the menu never renders invisible.
fn style(config: &config::Config, pal: Option<&palette::Palette>) -> MenuStyle {
    let d = config.panel.sensor_defaults.as_ref();
    let tone = |pal_value: Option<&str>, configured: Option<&str>, fallback: Color| -> Color {
        pal_value
            .and_then(|p| config::parse_color(&palette::keep_config_alpha(p, configured)).ok())
            .or_else(|| configured.and_then(|c| config::parse_color(c).ok()))
            .unwrap_or(fallback)
    };
    let bg = tone(
        pal.map(|p| p.card.as_str()),
        d.and_then(|s| s.bg_color.as_deref()),
        panel_background(config, pal),
    );
    let num = |value: Option<u32>| value.unwrap_or(0) as u16;
    MenuStyle {
        bg,
        bg_hover: tone(
            pal.map(|p| p.card_hover.as_str()),
            d.and_then(|s| s.bg_color_hover.as_deref()),
            bg,
        ),
        text: tone(
            pal.map(|p| p.text.as_str()),
            d.and_then(|s| s.text_color.as_deref()),
            default_text(&config.panel, pal),
        ),
        border: d
            .and_then(|s| s.border_color.as_deref())
            .and_then(|c| config::parse_color(c).ok())
            .unwrap_or(Color::TRANSPARENT),
        border_width: num(d.and_then(|s| config::style_num(s.border_width))),
        border_radius: num(d.and_then(|s| config::style_num(s.border_radius))),
        border_type: d
            .and_then(|s| s.border_type.clone())
            .unwrap_or_else(|| "full".to_string()),
        padding: num(d.and_then(|s| config::style_num(s.padding))),
    }
}

fn position(config: &config::Config) -> WindowMenuPosition {
    config
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.window_menu.as_ref())
        .map(|w| w.position)
        .unwrap_or_default()
}

pub(crate) fn enabled(config: &config::Config) -> bool {
    config
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.window_menu.as_ref())
        .is_some_and(|w| w.enabled)
}

/// The idle seconds that hide the menu, `None` while the tweak or its
/// toggle is off.
pub(crate) fn idle_timeout(config: &config::Config) -> Option<u32> {
    config
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.window_menu.as_ref())
        .filter(|w| w.enabled && w.hide_idle)
        .map(|w| w.hide_after)
}

/// The menu's top-left in output-local px: just above or below the
/// window at `pos`, its left or right edge aligned with the window's
/// (never hanging past it), or inside the window when that spot would
/// leave the output.
pub(crate) fn place(
    rect: (i32, i32, i32, i32),
    size: (u32, u32),
    pos: WindowMenuPosition,
    screen: (u32, u32),
) -> (i32, i32) {
    let (x, y, w, h) = rect;
    let (mw, mh) = (size.0 as i32, size.1 as i32);
    let m = MARGIN;
    // Horizontal: the edge the position names, flush with the window's.
    let (left, right, center) = (x, x + w - mw, x + (w - mw) / 2);
    let outside = match pos {
        WindowMenuPosition::TopLeft => (left, y - mh - m),
        WindowMenuPosition::TopCenter => (center, y - mh - m),
        WindowMenuPosition::TopRight => (right, y - mh - m),
        WindowMenuPosition::BottomLeft => (left, y + h + m),
        WindowMenuPosition::BottomCenter => (center, y + h + m),
        WindowMenuPosition::BottomRight => (right, y + h + m),
    };
    let fits = outside.0 >= 0
        && outside.1 >= 0
        && outside.0 + mw <= screen.0 as i32
        && outside.1 + mh <= screen.1 as i32;
    let spot = if fits {
        outside
    } else {
        // A window against the screen edge has no room above/below:
        // drop the menu inside it, on the same side and still aligned.
        let inside_y = match pos {
            WindowMenuPosition::TopLeft
            | WindowMenuPosition::TopCenter
            | WindowMenuPosition::TopRight => y + m,
            WindowMenuPosition::BottomLeft
            | WindowMenuPosition::BottomCenter
            | WindowMenuPosition::BottomRight => y + h - mh - m,
        };
        (outside.0, inside_y)
    };
    // Last guard: an output smaller than the window itself.
    (
        spot.0.clamp(0, (screen.0 as i32 - mw).max(0)),
        spot.1.clamp(0, (screen.1 as i32 - mh).max(0)),
    )
}

/// The side of the menu surface that carries the transparent strip
/// reaching the window edge (the pill itself stays `MARGIN` away).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Bridge {
    Top,
    Bottom,
}

/// Where the menu surface goes: its top-left, the pill size and the
/// bridge side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Placed {
    /// Output-local top-left of the whole surface (strip included).
    pub(crate) at: (i32, i32),
    /// The pill's own size; the strip adds `MARGIN` on `bridge`'s side.
    pub(crate) size: (u32, u32),
    pub(crate) bridge: Option<Bridge>,
}

impl Placed {
    fn height(&self) -> u32 {
        self.size.1 + if self.bridge.is_some() { MARGIN as u32 } else { 0 }
    }
}

/// The surface for a window: the pill from `place` plus a transparent
/// `MARGIN` strip on the side facing the window. The strip makes the
/// surface touch the window edge, so the pointer leaves the window
/// straight onto the menu — a layer surface never takes the focus, and
/// `input:follow_mouse` can no longer jump to the window underneath
/// while the cursor is on its way to the buttons. Inside the window
/// (the screen-edge fallback) there is no strip: the pointer is already
/// over the window.
pub(crate) fn placed(
    rect: (i32, i32, i32, i32),
    screen: (u32, u32),
    pos: WindowMenuPosition,
    size: (u32, u32),
) -> Placed {
    let pill = place(rect, size, pos, screen);
    let (_, y, _, h) = rect;
    let outside = match pos {
        WindowMenuPosition::TopLeft
        | WindowMenuPosition::TopCenter
        | WindowMenuPosition::TopRight => pill.1 + size.1 as i32 <= y,
        WindowMenuPosition::BottomLeft
        | WindowMenuPosition::BottomCenter
        | WindowMenuPosition::BottomRight => pill.1 >= y + h,
    };
    let bridge = outside.then_some(match pos {
        WindowMenuPosition::TopLeft
        | WindowMenuPosition::TopCenter
        | WindowMenuPosition::TopRight => Bridge::Bottom,
        WindowMenuPosition::BottomLeft
        | WindowMenuPosition::BottomCenter
        | WindowMenuPosition::BottomRight => Bridge::Top,
    });
    let at = match bridge {
        Some(Bridge::Top) => (pill.0, pill.1 - MARGIN),
        _ => pill,
    };
    Placed { at, size, bridge }
}

/// Ease-out cubic: quick start, soft landing.
fn eased(from: (i32, i32), to: (i32, i32), t: f32) -> (i32, i32) {
    let k = 1.0 - (1.0 - t).powi(3);
    let lerp = |a: i32, b: i32| a + ((b - a) as f32 * k).round() as i32;
    (lerp(from.0, to.0), lerp(from.1, to.1))
}

/// The margin placing the surface at `at` (anchored top-left).
fn margin_at(at: (i32, i32)) -> Margin {
    Margin {
        top: at.1,
        left: at.0,
        ..Default::default()
    }
}

/// The tick's reconcile: create the surface when the tweak turns on,
/// close it when it turns off, and re-read the active window — events
/// keep it prompt, this makes it self-healing.
pub(crate) fn sync(panel: &mut Panel) -> Task<Message> {
    if !enabled(&panel.config) {
        return match panel.window_menu.take() {
            Some(menu) => iced::window::close(menu.id),
            None => Task::none(),
        };
    }
    if idle_timeout(&panel.config).is_none() {
        if let Some(menu) = panel.window_menu.as_mut() {
            menu.fade = None;
            menu.alpha = 1.0;
        }
    }
    // A glide in flight lands on its own; a tick must not snap it.
    if panel.window_menu.as_ref().is_some_and(|menu| menu.from.is_some()) {
        return Task::none();
    }
    // Re-read every second: events keep this prompt, the tick makes it
    // self-healing (a re-tile that emits no event still re-places it).
    apply(panel, active_target())
}

/// The compositor's idle threshold for the menu: fade out after the
/// configured silence, fade back in on the next input.
pub(crate) fn idle(panel: &mut Panel, idled: bool) -> Task<Message> {
    let Some(menu) = panel.window_menu.as_mut() else {
        return Task::none();
    };
    let now = Instant::now();
    let from = menu.alpha();
    let to = if idled { 0.0 } else { 1.0 };
    menu.alpha = from;
    if (from - to).abs() < 0.001 {
        menu.fade = None;
        menu.alpha = to;
        return Task::none();
    }
    menu.fade = Some(Fade {
        from,
        to,
        started: now,
        duration: if idled { FADE_OUT } else { FADE_IN },
    });
    Task::none()
}

/// Apply one window state change from the socket thread.
pub(crate) fn apply(panel: &mut Panel, target: Option<Target>) -> Task<Message> {
    let Some(target) = target else {
        // Nothing focused (desktop, no windows): the menu goes away.
        return match panel.window_menu.take() {
            Some(menu) => iced::window::close(menu.id),
            None => Task::none(),
        };
    };
    let size = menu_size(style(&panel.config, panel.palette.as_ref()).padding);
    // A hidden fullscreen window has no border to attach to.
    if target.fullscreen {
        return match panel.window_menu.take() {
            Some(menu) => iced::window::close(menu.id),
            None => Task::none(),
        };
    }
    let Some(menu) = panel.window_menu.as_mut() else {
        return open(panel, target, size);
    };
    if menu.size != size || menu.output.as_deref() != Some(target.output.as_str()) {
        return reopen(panel, target, size);
    }
    let focus_changed = menu.target.as_ref().map(|t| t.address.as_str()) != Some(target.address.as_str());
    let to = placed(
        target.rect,
        target.screen,
        position(&panel.config),
        size,
    );
    let bridge = menu.bridge;
    menu.target = Some(target);
    // A strip that appears/disappears changes the surface height: only a
    // new surface has it (the window reached the screen edge).
    if bridge != to.bridge {
        let target = menu.target.clone();
        return match target {
            Some(target) => reopen(panel, target, size),
            None => Task::none(),
        };
    }
    let to = to.at;
    if focus_changed {
        // Glide from where the menu is now; the frames carry it.
        menu.from = Some((menu.at, Instant::now()));
        return Task::none();
    }
    // The same window moved: snap (a drag would restart the glide).
    menu.from = None;
    if menu.at == to {
        return Task::none();
    }
    menu.at = to;
    Task::done(Message::MarginChange {
        id: menu.id,
        margin: margin_at(to),
    })
}

/// One animation frame: interpolate towards the current target.
pub(crate) fn frame(panel: &mut Panel) -> Task<Message> {
    let now = Instant::now();
    let Some(menu) = panel.window_menu.as_mut() else {
        return Task::none();
    };
    if let Some(fade) = menu.fade {
        if fade.done(now) {
            menu.alpha = fade.to;
            menu.fade = None;
        }
    }
    let Some(glide) = menu.from else {
        return Task::none();
    };
    let Some(target) = menu.target.as_ref() else {
        menu.from = None;
        return Task::none();
    };
    let to = placed(
        target.rect,
        target.screen,
        position(&panel.config),
        menu.size,
    )
    .at;
    let t = glide.1.elapsed().as_secs_f32() / ANIM.as_secs_f32();
    let at = if t >= 1.0 {
        menu.from = None;
        to
    } else {
        eased(glide.0, to, t)
    };
    menu.at = at;
    Task::done(Message::MarginChange {
        id: menu.id,
        margin: margin_at(at),
    })
}

/// Create the surface for `target`.
fn open(panel: &mut Panel, target: Target, size: (u32, u32)) -> Task<Message> {
    let spot = placed(target.rect, target.screen, position(&panel.config), size);
    let output = target.output.clone();
    let (id, command) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(spot.size.0, spot.height()),
        layer: Layer::Overlay,
        // Explicit top-left + margins: the position is a plain pixel
        // offset that the glide can interpolate.
        anchor: Anchor::Top | Anchor::Left,
        // -1: the bar's own reserved strip must not shift the menu.
        exclusive_zone: Some(-1),
        margin: Some(margin_at(spot.at)),
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: OutputOption::OutputName(output.clone()),
        namespace: Some("piuma-window-menu".to_string()),
        ..Default::default()
    });
    panel.window_menu = Some(WindowMenu {
        id,
        target: Some(target),
        hover: None,
        from: None,
        alpha: 1.0,
        fade: None,
        output: Some(output),
        at: spot.at,
        size,
        bridge: spot.bridge,
    });
    command
}

/// Close the surface and open it again (output or size changed).
fn reopen(panel: &mut Panel, target: Target, size: (u32, u32)) -> Task<Message> {
    let close = panel.window_menu.take().map(|menu| menu.id);
    let open = open(panel, target, size);
    match close {
        Some(id) => Task::batch([iced::window::close(id), open]),
        None => open,
    }
}

/// Dispatch one action against a window (both menus carry the same
/// three; `floating`/`pinned` let Sticky float a tiled window first).
pub(crate) fn run(action: Action, address: &str, floating: bool, pinned: bool) {
    match action {
        Action::Close => windows::close_window(address),
        Action::Float => windows::toggle_float(address),
        Action::Sticky => windows::toggle_sticky(address, floating, pinned),
    }
}

/// The menu row: three icon buttons in the sensor's own box.
pub(crate) fn view<'a>(panel: &'a Panel, menu: &'a WindowMenu) -> Element<'a, Message> {
    let st = style(&panel.config, panel.palette.as_ref());
    // The idle fade dims the whole pill, icons included.
    let alpha = menu.alpha();
    let faded = |color: Color| Color {
        a: color.a * alpha,
        ..color
    };
    let pinned = menu.target.as_ref().is_some_and(|t| t.pinned);
    let theme = panel
        .config
        .panel
        .icon_theme
        .as_deref()
        .unwrap_or_else(|| icons::icon_theme());
    let radius = (st.border_radius as f32).min(ICON as f32 / 2.0);
    let hover_bg = faded(st.bg_hover);
    let mut row = Row::new().spacing(GAP as f32).align_y(Alignment::Center);
    for action in Action::ALL {
        // `center_*` also sets the axis to Fill, which a shrink row would
        // measure as zero: align instead, the box keeps its fixed edge.
        let cell = container(icon_widget(theme, action, pinned, faded(st.text), alpha))
            .width(Length::Fixed(ICON as f32))
            .height(Length::Fixed(ICON as f32))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(move |_theme| container::Style {
                background: (menu.hover == Some(action))
                    .then_some(Background::Color(hover_bg)),
                border: Border {
                    radius: radius.into(),
                    ..Default::default()
                },
                ..container::Style::default()
            });
        row = row.push(
            MouseArea::new(cell)
                .on_press(Message::WindowMenuAction(action))
                .on_enter(Message::WindowMenuHover(Some(action)))
                // The exit names its row: a stale one must not clear the
                // hover the next row's enter just set (widget order).
                .on_exit(Message::WindowMenuLeft(action))
                .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    let pill = crate::styled_box(
        row.into(),
        Some(faded(st.bg)),
        Some(faded(st.border)),
        st.border_width,
        st.border_radius,
        &st.border_type,
        [false; 4],
        Padding::from(st.padding as f32),
        // The surface is exactly this size: fill it (a shrink box would
        // leave the pill smaller than the surface it was created for).
        Length::Fixed(menu.size.0 as f32),
        Length::Fixed(menu.size.1 as f32),
    );
    // The strip stays empty: the pill keeps its `MARGIN` gap from the
    // window, the surface just reaches it.
    let pad = |top: bool| iced::Padding {
        top: if top { MARGIN as f32 } else { 0.0 },
        right: 0.0,
        bottom: if top { 0.0 } else { MARGIN as f32 },
        left: 0.0,
    };
    let (width, height) = (
        menu.size.0 as f32,
        menu.size.1 as f32
            + if menu.bridge.is_some() { MARGIN as f32 } else { 0.0 },
    );
    let padding = match menu.bridge {
        Some(Bridge::Top) => pad(true),
        Some(Bridge::Bottom) => pad(false),
        None => iced::Padding::ZERO,
    };
    container(pill)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .padding(padding)
        .into()
}

/// The action's theme icon, tinted to the menu's text colour; a theme
/// without it falls back to a letter (never an empty button).
fn icon_widget(
    theme: &str,
    action: Action,
    pinned: bool,
    color: Color,
    opacity: f32,
) -> Element<'static, Message> {
    match icons::resolve(theme, action.icon(pinned), ICON as u16) {
        Some(path) if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) => {
            svg(svg::Handle::from_path(path))
                .width(Length::Fixed(ICON as f32))
                .height(Length::Fixed(ICON as f32))
                .opacity(opacity)
                .style(move |_theme, _status| svg::Style { color: Some(color) })
                .into()
        }
        Some(path) => image(image::Handle::from_path(path))
            .width(Length::Fixed(ICON as f32))
            .height(Length::Fixed(ICON as f32))
            .opacity(opacity)
            .into(),
        None => text(action.letter())
            .size(ICON as f32)
            .color(color)
            .into(),
    }
}

// ── Hyprland sockets ────────────────────────────────────────────────

/// Hyprland's IPC paths inside `$XDG_RUNTIME_DIR/hypr/<signature>/`.
fn socket(name: &str) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let runtime = Path::new(&runtime);
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            // Not launched by Hyprland: the session directory is the only
            // one under `hypr/` (never more than one live compositor).
            std::fs::read_dir(runtime.join("hypr"))
                .ok()?
                .filter_map(|entry| entry.ok())
                .next()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })?;
    Some(runtime.join("hypr").join(signature).join(name))
}

/// One request on the persistent socket (`j/activewindow`, …): the
/// compositor answers and closes, so this is a couple of hundred µs
/// with no process spawn.
fn query(command: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket(".socket.sock")?).ok()?;
    stream.write_all(command.as_bytes()).ok()?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).ok()?;
    Some(String::from_utf8_lossy(&reply).into_owned())
}

/// The focused window and the output holding it (two tiny reads).
pub(crate) fn active_target() -> Option<Target> {
    parse_target(&query("j/activewindow")?, &query("j/monitors")?)
}

/// Parse `j/activewindow` against the monitor list: the window's output
/// is the one holding its centre, and the rect goes output-local.
pub(crate) fn parse_target(window: &str, monitors: &str) -> Option<Target> {
    let w: serde_json::Value = serde_json::from_str(window).ok()?;
    let monitors: Vec<serde_json::Value> = serde_json::from_str(monitors).ok()?;
    let address = w.get("address")?.as_str()?.to_string();
    let pair = |key: &str| -> Option<(i32, i32)> {
        let list = w.get(key)?.as_array()?;
        Some((list.first()?.as_i64()? as i32, list.get(1)?.as_i64()? as i32))
    };
    let (x, y) = pair("at")?;
    let (width, height) = pair("size")?;
    let (cx, cy) = (x + width / 2, y + height / 2);
    let monitor = monitors.iter().find(|m| {
        let num = |key: &str| m.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        cx >= num("x") && cx < num("x") + num("width") && cy >= num("y") && cy < num("y") + num("height")
    })?;
    let num = |key: &str| monitor.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let (mx, my) = (num("x"), num("y"));
    Some(Target {
        address,
        rect: (x - mx, y - my, width, height),
        screen: (num("width").max(1) as u32, num("height").max(1) as u32),
        output: monitor.get("name")?.as_str()?.to_string(),
        floating: w.get("floating").and_then(|v| v.as_bool()).unwrap_or(false),
        pinned: w.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false),
        fullscreen: w.get("fullscreen").and_then(|v| v.as_i64()).unwrap_or(0) > 0,
    })
}

/// Events that change what the menu must show (focus, geometry, flags).
fn relevant(line: &str) -> bool {
    [
        "activewindow>>",
        "openwindow>>",
        "closewindow>>",
        "movewindow>>",
        "resizewindow>>",
        "changefloatingmode>>",
        "fullscreen>>",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

/// The focus subscription: one thread blocked on Hyprland's event
/// socket, idle cost zero.
pub(crate) fn subscription() -> iced::Subscription<Message> {
    iced::Subscription::run(|| iced::stream::channel(32, run_listener))
}

async fn run_listener(sender: iced::futures::channel::mpsc::Sender<Message>) {
    smol::unblock(move || listen(sender)).await;
}

/// Read events forever, reconnecting after a compositor restart or a
/// socket error; a failed send means the subscription is gone.
fn listen(mut sender: iced::futures::channel::mpsc::Sender<Message>) {
    loop {
        if let Some(path) = socket(".socket2.sock") {
            if let Ok(stream) = UnixStream::connect(path) {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if !relevant(&line) {
                        continue;
                    }
                    if sender
                        .try_send(Message::WindowMenuSync(active_target()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Above or below the window; the side positions sit flush with the
    /// window's own edge instead of hanging outside it.
    /// Outside the window the surface grows a `MARGIN` strip towards it
    /// (the pill keeps its gap); inside, the surface is just the pill.
    #[test]
    fn surface_bridges_the_gap_towards_the_window() {
        let rect = (100, 100, 400, 300);
        let size = (60, 22);
        let screen = (1920, 1080);
        // Above the window: strip on the surface's bottom.
        let p = placed(rect, screen, WindowMenuPosition::TopLeft, size);
        assert_eq!(p.at, (100, 74), "surface still starts at the pill");
        assert_eq!((p.size, p.height()), (size, 22 + 4));
        assert_eq!(p.bridge, Some(Bridge::Bottom));
        // Below it: the strip is on top, so the surface starts 4 px higher.
        let p = placed(rect, screen, WindowMenuPosition::BottomRight, size);
        assert_eq!(p.at, (440, 400), "surface reaches the window's bottom edge");
        assert_eq!(p.height(), 26);
        assert_eq!(p.bridge, Some(Bridge::Top));
        // Inside (a window flush with the screen edge): no strip.
        let p = placed((0, 0, 1920, 1080), screen, WindowMenuPosition::TopLeft, size);
        assert_eq!(p.at, (0, MARGIN), "the pill, no bridge");
        assert_eq!((p.height(), p.bridge), (22, None));
    }

    #[test]
    fn place_puts_the_menu_outside_the_configured_edge() {
        let rect = (100, 100, 400, 300);
        let size = (60, 22);
        let screen = (1920, 1080);
        let at = |p| place(rect, size, p, screen);
        assert_eq!(at(WindowMenuPosition::TopLeft), (100, 74), "flush left");
        assert_eq!(at(WindowMenuPosition::TopCenter), (270, 74));
        assert_eq!(at(WindowMenuPosition::TopRight), (440, 74), "flush right");
        assert_eq!(at(WindowMenuPosition::BottomLeft), (100, 404));
        assert_eq!(at(WindowMenuPosition::BottomCenter), (270, 404));
        assert_eq!(at(WindowMenuPosition::BottomRight), (440, 404));
    }

    /// A window flush with the output edge flips the menu inside it.
    #[test]
    fn place_flips_inside_when_the_outside_spot_leaves_the_output() {
        let size = (60, 22);
        let screen = (1920, 1080);
        // Maximized window: every outside spot is off-screen.
        let rect = (0, 0, 1920, 1080);
        assert_eq!(
            place(rect, size, WindowMenuPosition::TopLeft, screen),
            (0, MARGIN),
            "inside the window, still flush with its left edge"
        );
        assert_eq!(
            place(rect, size, WindowMenuPosition::TopRight, screen),
            (1920 - 60, MARGIN)
        );
        assert_eq!(
            place(rect, size, WindowMenuPosition::BottomRight, screen),
            (1920 - 60, 1080 - 22 - MARGIN)
        );
        assert_eq!(
            place(rect, size, WindowMenuPosition::BottomLeft, screen),
            (0, 1080 - 22 - MARGIN)
        );
        // A narrow window keeps its alignment: the menu may be wider than
        // the window, but the edge it names still meets the window's.
        let rect = (100, 200, 40, 400);
        assert_eq!(
            place(rect, size, WindowMenuPosition::TopLeft, screen),
            (100, 174),
            "left edge aligned even when the menu is wider"
        );
        assert_eq!(
            place(rect, size, WindowMenuPosition::TopRight, screen),
            (80, 174),
            "right edges meet at 140"
        );
    }

    /// The glide starts at the old spot, ends exactly on the new one.
    #[test]
    fn eased_glides_and_lands_on_the_target() {
        assert_eq!(eased((0, 0), (100, 50), 0.0), (0, 0));
        assert_eq!(eased((0, 0), (100, 50), 1.0), (100, 50));
        let mid = eased((0, 0), (100, 100), 0.5);
        assert!(mid.0 > 50 && mid.0 < 100, "ease-out front-loads: {mid:?}");
    }

    #[test]
    fn menu_size_adds_the_padding_around_three_icons() {
        assert_eq!(menu_size(0), (3 * ICON + 2 * GAP, ICON));
        let (w, h) = menu_size(3);
        assert_eq!((w, h), (3 * ICON + 2 * GAP + 6, ICON + 6));
    }

    #[test]
    fn sticky_and_float_target_the_pressed_window() {
        assert_eq!(
            Action::Sticky.icon(false),
            "window-pin",
            "unpinned shows the pin"
        );
        assert_eq!(Action::Sticky.icon(true), "window-unpin");
        assert_eq!(Action::Close.icon(false), "window-close");
        assert_eq!(Action::Float.icon(false), "window-duplicate");
    }

    /// The target parses the output-local rect, the output's size and
    /// the flags; nothing focused is an empty object (`None`).
    #[test]
    fn parse_target_reads_geometry_and_flags() {
        let window = r#"{"address":"0xAB12","at":[500,200],"size":[800,600],
            "floating":true,"pinned":true,"fullscreen":0}"#;
        let monitors = r#"[{"name":"eDP-1","x":0,"y":0,"width":1920,"height":1080},
                           {"name":"HDMI-A-2","x":1920,"y":0,"width":2560,"height":1440}]"#;
        let t = parse_target(window, monitors).expect("focused window");
        assert_eq!(t.address, "0xAB12");
        assert_eq!(t.rect, (500, 200, 800, 600), "output-local rect");
        assert_eq!(t.screen, (1920, 1080));
        assert_eq!(t.output, "eDP-1");
        assert!(t.floating && t.pinned && !t.fullscreen);

        let t = parse_target(window, monitors).unwrap();
        assert_eq!(t.output, "eDP-1", "the monitor holding the centre wins");
        assert!(parse_target("{}", monitors).is_none(), "nothing focused");
        assert!(parse_target(window, "not json").is_none());
    }

    /// The second output's windows land on its own surface bounds.
    #[test]
    fn parse_target_converts_global_rects_per_output() {
        let window = r#"{"address":"0x1","at":[2000,100],"size":[1000,800],
            "floating":false,"pinned":false,"fullscreen":0}"#;
        let monitors = r#"[{"name":"eDP-1","x":0,"y":0,"width":1920,"height":1080},
                           {"name":"HDMI-A-2","x":1920,"y":0,"width":2560,"height":1440}]"#;
        let t = parse_target(window, monitors).unwrap();
        assert_eq!(t.rect, (80, 100, 1000, 800));
        assert_eq!(t.screen, (2560, 1440));
        assert_eq!(t.output, "HDMI-A-2");
    }

    /// The fades are linear: ten seconds out and two back in, with the
    /// endpoints exact.
    #[test]
    fn fades_reach_their_endpoints_linearly() {
        let start = Instant::now();
        let out = Fade {
            from: 1.0,
            to: 0.0,
            started: start,
            duration: FADE_OUT,
        };
        assert!((out.alpha(start) - 1.0).abs() < 1e-6);
        assert!((out.alpha(start + Duration::from_secs(5)) - 0.5).abs() < 1e-6);
        assert!((out.alpha(start + FADE_OUT) - 0.0).abs() < 1e-6);
        assert!(out.done(start + FADE_OUT));

        let back = Fade {
            from: 0.2,
            to: 1.0,
            started: start,
            duration: FADE_IN,
        };
        assert!((back.alpha(start + Duration::from_secs(1)) - 0.6).abs() < 1e-6);
        assert!(!back.done(start + Duration::from_secs(1)));
    }

    #[test]
    fn relevant_events_cover_focus_and_geometry() {
        assert!(relevant("activewindow>>0x1,kitty"));
        assert!(relevant("movewindow>>0x1,10,20"));
        assert!(relevant("resizewindow>>0x1,800,600"));
        assert!(relevant("closewindow>>0x1"));
        assert!(!relevant("workspace>>3"));
        assert!(!relevant("openlayer>>piuma,bar"));
    }
}
