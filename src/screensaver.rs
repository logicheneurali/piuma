//! The lock screen: full-screen palette rectangles that shift at their
//! own pace, opened after the idle timeout or from the `lock` sensor.
//! It uses `ext-session-lock-v1` when the compositor supports it, so the
//! compositor hides every window and ignores its normal keybinds while
//! locked; without the protocol it falls back to a layer-shell overlay.
//! Either way it is dismissed only by the session password (PAM,
//! `auth.rs`): input no longer closes it.

use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path};
use iced::widget::button::{Status as ButtonStatus, Style as ButtonStyle};
use iced::widget::{button, container, text, Column, Stack};
use iced::{Alignment, Color, Element, Length, Point, Size};

use crate::config;
use crate::{monitor_names, styles, Command, Message, Panel};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;
use iced_exwlshell::shell;

/// The grid: 35 rows by 25 columns across the output, whatever its size
/// (each cell is one pixel of the image stretched to the surface, so the
/// rectangles come out ~77×31 px on a 1920×1080 output).
pub(crate) const COLS: u32 = 25;
pub(crate) const ROWS: u32 = 35;
/// Animation cadence while the overlay is up (5 fps: the colours drift,
/// a full-screen redraw does not need more). Zero frames while it is off.
pub(crate) const FRAME: Duration = Duration::from_millis(200);

/// `SignalScreensaver` from the setup's button: start it now, whatever
/// the idle state.
pub(crate) fn signal_subscription() -> iced::Subscription<Message> {
    iced::Subscription::run(|| {
        iced::stream::channel(4, |sender| async move {
            smol::unblock(move || listen_signal(sender)).await;
        })
    })
}

fn listen_signal(mut sender: iced::futures::channel::mpsc::Sender<Message>) {
    let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGUSR1])
    else {
        return;
    };
    for _ in signals.forever() {
        if sender.try_send(Message::ScreensaverManual).is_err() {
            return; // the app is gone
        }
    }
}

/// Which protocol holds the session: `ext-session-lock-v1` when the
/// compositor has it, the layer-shell overlay otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LockMode {
    /// No lock requested.
    #[default]
    Off,
    /// `Lock` sent, waiting for the compositor to answer.
    Requested,
    /// The compositor granted the session lock.
    Session,
    /// The compositor has no `ext-session-lock-v1`: layer overlay.
    Fallback,
}

/// Lock now: the idle trigger and the `lock` sensor's click alike (a
/// manual lock works even with the idle tweak off). A screen with no PAM
/// stack would lock with no way back, so it refuses to open then.
pub(crate) fn show(panel: &mut Panel) -> Command<Message> {
    if panel.screensaver_on {
        return Command::none();
    }
    if !crate::auth::available() {
        eprintln!("[piuma] lock: no PAM service in /etc/pam.d, not locking");
        return Command::none();
    }
    panel.screensaver_on = true;
    panel.screensaver_lock = LockMode::Requested;
    panel.screensaver_start = Some(Instant::now());
    panel.screensaver_wait = false;
    panel.screensaver_retry = None;
    panel.screensaver_password.clear();
    panel.screensaver_error = None;
    panel.screensaver_pending = false;
    panel.screensaver_prompt_open = false;
    panel.screensaver.clear();
    // Every cell starts at its own phase, then the brightness of its
    // colour drives its pace (see `advance`).
    panel.screensaver_phases = initial_phases();
    panel.screensaver_last = Some(Instant::now());
    // The compositor creates one lock surface per output; `on_shell`
    // adopts them, or `LockDenied` opens the layer fallback.
    Command::done(Message::Lock)
}

/// Take the lock down and forget the idle state. The session-lock
/// surfaces belong to the compositor: the unlock request destroys them.
pub(crate) fn close(panel: &mut Panel) -> Command<Message> {
    let mode = std::mem::replace(&mut panel.screensaver_lock, LockMode::Off);
    panel.screensaver_on = false;
    panel.screensaver_start = None;
    panel.screensaver_wait = false;
    panel.screensaver_retry = None;
    panel.screensaver_phases.clear();
    panel.screensaver_last = None;
    panel.screensaver_password.clear();
    panel.screensaver_error = None;
    panel.screensaver_pending = false;
    panel.screensaver_prompt_open = false;
    let mut cmds: Vec<Command<Message>> = Vec::new();
    match mode {
        // `UnLock` on a request the compositor never granted is harmless.
        LockMode::Requested | LockMode::Session => cmds.push(Command::done(Message::UnLock)),
        LockMode::Fallback => cmds.extend(
            panel
                .screensaver
                .drain(..)
                .map(|(_, id)| iced::window::close(id)),
        ),
        LockMode::Off => {}
    }
    Command::batch(cmds)
}

/// One `ShellEvent` from the runtime: the session-lock surfaces it
/// creates for our request, the compositor's answer, their removal. The
/// output name is irrelevant here (the compositor places each lock
/// surface itself), so session entries carry an empty one.
pub(crate) fn on_shell(panel: &mut Panel, event: shell::ShellEvent) -> Command<Message> {
    use shell::ShellEvent as E;
    match event {
        E::NewShell(info) => {
            if info.shell == shell::ShellType::SessionLock
                && !panel.screensaver.iter().any(|(_, id)| *id == info.window) {
                    panel.screensaver.push((String::new(), info.window));
                }
            Command::none()
        }
        E::Closed(window) => {
            panel.screensaver.retain(|(_, id)| *id != window);
            Command::none()
        }
        E::Locked => {
            if panel.screensaver_lock == LockMode::Requested {
                panel.screensaver_lock = LockMode::Session;
            }
            Command::none()
        }
        E::LockDenied => {
            if panel.screensaver_lock != LockMode::Requested {
                return Command::none();
            }
            eprintln!(
                "[piuma] lock: ext-session-lock-v1 not available, using a layer overlay"
            );
            panel.screensaver_lock = LockMode::Fallback;
            sync(panel)
        }
        E::LockedFinished => {
            panel.screensaver_lock = LockMode::Off;
            panel.screensaver.clear();
            Command::none()
        }
        _ => Command::none(),
    }
}

/// The tick's reconciliation: once the feature is off (or the overlay
/// closed) every surface is closed; while it is up, a monitor plugged in
/// later joins and a removed one closes. The fullscreen retry rides here
/// too, at most one check a minute.
pub(crate) fn sync(panel: &mut Panel) -> Command<Message> {
    let mut cmds: Vec<Command<Message>> = Vec::new();
    if panel.config.screensaver().is_none() {
        // The idle trigger is off; a manual lock (sensor or signal) stays
        // until the password unlocks it.
        panel.screensaver_wait = false;
        panel.screensaver_retry = None;
    }
    // Waiting on a fullscreen window: re-check once a minute so a video
    // that leaves fullscreen on its own still ends with the overlay.
    if panel.screensaver_wait
        && !panel.screensaver_on
        && panel
            .screensaver_retry
            .is_none_or(|at| Instant::now() >= at)
    {
        panel.screensaver_retry = Some(Instant::now() + Duration::from_secs(60));
        cmds.push(Command::perform(
            smol::unblock(|| {
                crate::hypr::active_is_fullscreen() == Some(true)
                    || crate::playback::blocks_screensaver()
            }),
            Message::ScreensaverBlocked,
        ));
    }
    if !panel.screensaver_on {
        return Command::batch(cmds);
    }
    // The session lock drives its surfaces itself (one per output, plus
    // the ones for monitors plugged in while locked): nothing to open
    // or reconcile here.
    if panel.screensaver_lock != LockMode::Fallback {
        return Command::batch(cmds);
    }
    let desired = monitor_names();
    let (to_open, to_close) = crate::wallpaper_ui::wallpaper_plan(&panel.screensaver, &desired);
    crate::wallpaper_ui::forget_closed(&mut panel.screensaver, &to_close);
    cmds.extend(to_close.into_iter().map(iced::window::close));
    for name in to_open {
        cmds.push(open_surface(panel, name));
    }
    Command::batch(cmds)
}

/// One overlay on `output`, used only when the compositor has no
/// `ext-session-lock-v1`. Every surface grabs the keyboard
/// (`Exclusive`): the card is drawn on all of them, so whichever one the
/// compositor focuses receives the password.
fn open_surface(panel: &mut Panel, output: String) -> Command<Message> {
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::FILL,
        layer: Layer::Overlay,
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        // Ignore the bar's reserved strip: the overlay covers the output.
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        output_option: OutputOption::OutputName(output.clone()),
        // Not transparent: while locked, clicks must not reach the apps
        // underneath.
        events_transparent: false,
        namespace: Some("piuma-screensaver".to_string()),
        ..Default::default()
    });
    panel.screensaver.push((output, id));
    open
}

pub(crate) fn is_window(panel: &Panel, window: iced::window::Id) -> bool {
    panel.screensaver.iter().any(|(_, id)| *id == window)
}

/// Whether this event means "I want to unlock": a click, a wheel or a
/// key. Pointer motion alone does not reveal the card.
pub(crate) fn reveal_input(event: &iced::Event) -> bool {
    matches!(
        event,
        iced::Event::Mouse(
            iced::mouse::Event::ButtonPressed { .. }
                | iced::mouse::Event::WheelScrolled { .. }
        ) | iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { .. })
    )
}

/// One animation frame: every cell's phase advances by `dt` divided by
/// the cycle length its CURRENT colour asks for, so bright cells change
/// in 3 s and dark ones take 10 (see [`cycle_seconds`]). Called from
/// the frame message, before the view reads the phases.
pub(crate) fn advance(panel: &mut Panel) {
    if !panel.screensaver_on {
        return;
    }
    let now = Instant::now();
    let dt = panel
        .screensaver_last
        .map(|last| now.duration_since(last).as_secs_f32())
        // A stall (suspend, a blocked compositor) must not jump the
        // whole grid: one long frame is enough.
        .map(|dt| dt.min(0.5))
        .unwrap_or(0.0);
    panel.screensaver_last = Some(now);
    let ramp = ramp(panel);
    advance_phases(&mut panel.screensaver_phases, dt, &ramp);
}

// ── view ────────────────────────────────────────────────────────────

/// The overlay: one rounded rectangle per cell, drawn on a canvas so
/// the radius is real screen pixels (4 px) instead of an upscaled
/// 25×35 texture — the grid stays crisp and the corners are rounded.
/// Every lock surface shows the password card once the user asks to
/// unlock, so the focused output always has the field.
pub(crate) fn view(panel: &Panel, _window: iced::window::Id) -> Element<'_, Message> {
    let grid = Canvas::new(Grid {
        phases: &panel.screensaver_phases,
        ramp: ramp(panel),
    })
    .width(Length::Fill)
    .height(Length::Fill);
    if !panel.screensaver_prompt_open {
        return grid.into();
    }
    // A dark scrim keeps the card readable over the moving grid.
    let scrim = container(iced::widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t| container::Style {
            background: Some(iced::Background::Color(Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.35,
            })),
            ..container::Style::default()
        });
    Stack::new()
        .push(grid)
        .push(scrim)
        .push(container(prompt_card(panel)).center(Length::Fill))
        .into()
}

/// The field's content: one dot per typed character (capped, so a very
/// long password cannot outgrow the fixed box), the placeholder while
/// empty.
fn password_mask<'a>(panel: &Panel, color: Color) -> Element<'a, Message> {
    let typed = panel.screensaver_password.chars().count();
    let content = if typed == 0 {
        text("Password".to_string())
            .size(16)
            .color(Color { a: 0.55, ..color })
    } else {
        text("•".repeat(typed.min(24))).size(16).color(color)
    };
    content.into()
}

/// One key while the card is open: printable characters append, Backspace
/// removes the last one, Enter submits. Keys reach the focused lock
/// surface through `event::listen`, so no widget focus is involved.
pub(crate) fn type_key(
    panel: &mut Panel,
    key: &iced::keyboard::Key,
    text: Option<&str>,
) -> Command<Message> {
    use iced::keyboard::key::Named;
    if !panel.screensaver_on || panel.screensaver_pending {
        return Command::none();
    }
    match key {
        iced::keyboard::Key::Named(Named::Enter) => {
            if panel.screensaver_password.is_empty() {
                Command::none()
            } else {
                Command::done(Message::ScreensaverSubmit)
            }
        }
        iced::keyboard::Key::Named(Named::Backspace) => {
            panel.screensaver_password.pop();
            Command::none()
        }
        _ => {
            if let Some(text) = text {
                let typed: String = text.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    panel.screensaver_password.push_str(&typed);
                }
            }
            Command::none()
        }
    }
}

/// The password card: title, the secure field, the last failure and the
/// Unlock button — every colour from the active palette (the wallpaper's
/// tones when on), so it reads as part of piuma. Enter in the field
/// submits too.
fn prompt_card(panel: &Panel) -> Element<'_, Message> {
    let pal = panel.palette.as_ref();
    let text_color = styles::default_text(&panel.config.panel, pal);
    let bg = opaque(styles::panel_background(&panel.config, pal));
    let accent = pal
        .and_then(|p| config::parse_color(&p.card_hover).ok())
        .map(opaque)
        .unwrap_or(text_color);
    let on_accent = if luma(accent) > 0.55 {
        Color::BLACK
    } else {
        Color::WHITE
    };
    let field_bg = mix(bg, text_color, 0.08);
    let body = Column::new()
        .push(text("Locked").size(30).color(text_color))
        .push(
            // A plain masked field, not an iced text input: the lock
            // surface has no IME/focus plumbing, and the mask needs no
            // widget state (keys come through `type_key`).
            container(password_mask(panel, text_color))
                .padding(10)
                .width(Length::Fixed(320.0))
                .style(move |_t| container::Style {
                    background: Some(iced::Background::Color(field_bg)),
                    border: iced::Border {
                        color: Color { a: 0.35, ..text_color },
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    ..container::Style::default()
                }),
        )
        .push(match (&panel.screensaver_error, panel.screensaver_pending) {
            (Some(error), _) => text(error)
                .size(14)
                .color(Color::from_rgb(1.0, 0.45, 0.45)),
            (None, true) => text("Checking…").size(14).color(text_color),
            (None, false) => text("Enter to unlock")
                .size(12)
                .color(Color { a: 0.7, ..text_color }),
        })
        .push(
            button(text("Unlock").size(16).color(on_accent))
                .on_press(Message::ScreensaverSubmit)
                .padding([10, 24])
                .style(move |_theme, status| ButtonStyle {
                    background: Some(iced::Background::Color(match status {
                        ButtonStatus::Hovered => Color { a: 0.88, ..accent },
                        ButtonStatus::Pressed => Color { a: 0.72, ..accent },
                        ButtonStatus::Disabled => Color { a: 0.4, ..accent },
                        ButtonStatus::Active => accent,
                    })),
                    text_color: on_accent,
                    border: iced::Border {
                        radius: 10.0.into(),
                        ..Default::default()
                    },
                    ..ButtonStyle::default()
                }),
        )
        .spacing(14)
        .align_x(Alignment::Center);
    container(body)
        .padding(28)
        .style(move |_t| container::Style {
            background: Some(iced::Background::Color(Color { a: 0.92, ..bg })),
            border: iced::Border {
                width: 1.0,
                color: Color { a: 0.3, ..text_color },
                radius: 14.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Corner radius of every cell, in screen pixels.
const RADIUS: f32 = 4.0;

struct Grid<'a> {
    phases: &'a [f32],
    ramp: [Color; 3],
}

impl canvas::Program<Message> for Grid<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // The palette's base tone fills the 4 px notches the rounded
        // corners leave between the cells.
        frame.fill_rectangle(bounds.position(), bounds.size(), self.ramp[0]);
        let cell = Size::new(
            bounds.width / COLS as f32,
            bounds.height / ROWS as f32,
        );
        let radius = iced::border::Radius {
            top_left: RADIUS,
            top_right: RADIUS,
            bottom_right: RADIUS,
            bottom_left: RADIUS,
        };
        for row in 0..ROWS as usize {
            for col in 0..COLS as usize {
                let i = row * COLS as usize + col;
                let phase = self.phases.get(i).copied().unwrap_or(0.0);
                let color = ramp_at(phase, &self.ramp);
                let top_left = Point::new(
                    bounds.x + col as f32 * cell.width,
                    bounds.y + row as f32 * cell.height,
                );
                frame.fill(
                    &Path::rounded_rectangle(top_left, cell, radius),
                    color,
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

/// The palette the rectangles walk: the wallpaper-derived tones when
/// active (base, card, accent), else the bar's own background with two
/// faint lifts of its text tone. Always opaque.
fn ramp(panel: &Panel) -> [Color; 3] {
    let bg = opaque(styles::panel_background(&panel.config, panel.palette.as_ref()));
    match panel.palette.as_ref() {
        Some(p) => {
            let tone = |hex: &str| config::parse_color(hex).map(opaque).unwrap_or(bg);
            [bg, tone(&p.card), tone(&p.card_hover)]
        }
        None => {
            let text = styles::default_text(&panel.config.panel, None);
            [bg, mix(bg, text, 0.10), mix(bg, text, 0.22)]
        }
    }
}

fn opaque(mut color: Color) -> Color {
    color.a = 1.0;
    color
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}

/// Every cell's starting phase: deterministic from its position, so the
/// grid opens varied instead of flashing in unison.
fn initial_phases() -> Vec<f32> {
    let n = (COLS * ROWS) as usize;
    (0..n)
        .map(|i| {
            let h = hash(i % COLS as usize, i / COLS as usize);
            ((h >> 8) & 0xffff) as f32 / 65535.0
        })
        .collect()
}

/// One full cycle at full brightness and at black: the ends of the
/// exponential curve below.
const FAST_SECONDS: f32 = 3.0;
const SLOW_SECONDS: f32 = 30.0;

/// Seconds for one full palette cycle: white (255,255,255) → 3 s,
/// black → 30 s, and the pace grows EXPONENTIALLY as the cell darkens
/// (`3 · 10^(1 − luma)`): a half-bright cell takes 9.5 s, not the 16.5 s
/// a straight line would give, while the darkest tones crawl for
/// 20–30 s. Most of the screen therefore sits on the dark end.
fn cycle_seconds(color: Color) -> f32 {
    FAST_SECONDS * (SLOW_SECONDS / FAST_SECONDS).powf(1.0 - luma(color))
}

/// Perceptual brightness (Rec. 709) of a colour, 0..1.
fn luma(color: Color) -> f32 {
    (0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b).clamp(0.0, 1.0)
}

/// Advance every phase by the time it needs, each at the pace of the
/// colour it is showing right now.
fn advance_phases(phases: &mut [f32], dt: f32, ramp: &[Color; 3]) {
    for phase in phases {
        let period = cycle_seconds(ramp_at(*phase, ramp)).max(0.1);
        *phase = (*phase + dt / period).rem_euclid(1.0);
    }
}

/// A smooth loop through the three stops (a → b → c → a): the phase
/// picks the segment and the smoothstep blends the two ends, so no
/// colour jumps.
fn ramp_at(x: f32, ramp: &[Color; 3]) -> Color {
    let scaled = x.rem_euclid(1.0) * 3.0;
    let i = (scaled.floor() as usize) % 3;
    let f = scaled.fract();
    let f = f * f * (3.0 - 2.0 * f);
    mix(ramp[i], ramp[(i + 1) % 3], f)
}

/// A tiny integer hash: stable per cell, spread across the bits.
fn hash(x: usize, y: usize) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp3() -> [Color; 3] {
        [
            Color::from_rgb(0.1, 0.1, 0.2),
            Color::from_rgb(0.2, 0.3, 0.4),
            Color::from_rgb(0.8, 0.5, 0.2),
        ]
    }

    /// The pace follows the cell's brightness: white takes 3 s, black
    /// 30, and the curve is exponential — a half-bright cell lands on
    /// 3·√10 ≈ 9.49 s, not the 16.5 s of a straight line.
    #[test]
    fn bright_cells_change_faster_than_dark_ones() {
        assert!((cycle_seconds(Color::WHITE) - 3.0).abs() < 1e-4);
        assert!((cycle_seconds(Color::BLACK) - 30.0).abs() < 1e-4);
        let grey = Color::from_rgb(0.5, 0.5, 0.5);
        let mid = cycle_seconds(grey);
        assert!((mid - 3.0 * 10.0_f32.sqrt()).abs() < 1e-3, "grey: {mid}");
        assert!(mid < 16.5, "the exponential pulls mid-tones faster");
        assert!(cycle_seconds(Color::from_rgb(0.9, 0.9, 0.9)) < mid);
        assert!(cycle_seconds(Color::from_rgb(0.1, 0.1, 0.1)) > mid);
    }

    /// Advancing a step moves a bright cell further than a dark one, all
    /// phases stay in 0..1, and an all-dark grid still moves.
    #[test]
    fn advance_moves_each_cell_at_its_own_pace() {
        let bright = [Color::WHITE, Color::WHITE, Color::WHITE];
        let dark = [Color::BLACK, Color::BLACK, Color::BLACK];
        let mut phases = [0.0_f32, 0.25, 0.5];
        advance_phases(&mut phases, 1.0, &bright);
        let bright_shift = phases[0];
        assert!(bright_shift > 0.30, "a white cell covers a third in 1 s");

        let mut phases = [0.0_f32, 0.25, 0.5];
        advance_phases(&mut phases, 1.0, &dark);
        assert!(phases[0] < 0.04, "a black cell takes 30 s per cycle");
        assert!(phases[0] > 0.0, "it still moves");

        let mut phases = [0.98_f32];
        advance_phases(&mut phases, 1.0, &bright);
        assert!((0.0..1.0).contains(&phases[0]), "the phase wraps");
    }

    /// The card appears on a click or a key, never on pointer motion
    /// alone: the idle lock shows just the grid until someone asks to
    /// unlock.
    #[test]
    fn the_card_reveals_on_click_or_key_not_on_motion() {
        use iced::mouse;
        assert!(reveal_input(&iced::Event::Mouse(
            mouse::Event::ButtonPressed(mouse::Button::Left)
        )));
        assert!(!reveal_input(&iced::Event::Mouse(
            mouse::Event::CursorMoved {
                position: iced::Point::ORIGIN
            }
        )));
        assert!(!reveal_input(&iced::Event::Mouse(
            mouse::Event::CursorEntered
        )));
    }

    /// Every cell starts at its own phase, deterministic from its
    /// position, so the grid opens varied instead of flashing in unison.
    #[test]
    fn initial_phases_are_varied_and_deterministic() {
        let phases = initial_phases();
        assert_eq!(phases.len(), (COLS * ROWS) as usize);
        assert_eq!(phases, initial_phases());
        assert!(phases.iter().all(|p| (0.0..1.0).contains(p)));
        assert!(
            phases[0] != phases[1],
            "neighbours do not start in lockstep"
        );
    }

    /// The ramp walk stays a pure function of the phase: same phase →
    /// same opaque colour, different phases land on different tones.
    #[test]
    fn ramp_walk_is_deterministic() {
        let ramp = ramp3();
        let a = ramp_at(0.3, &ramp);
        assert_eq!((a.r, a.g, a.b), {
            let b = ramp_at(0.3, &ramp);
            (b.r, b.g, b.b)
        });
        assert_eq!(a.a, 1.0);
        let b = ramp_at(0.7, &ramp);
        assert!((a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs() > 0.001);
        for phase in [0.0, 0.2, 0.5, 0.9, 1.0] {
            let c = ramp_at(phase, &ramp);
            for v in [c.r, c.g, c.b] {
                assert!((0.0..=1.0).contains(&v));
            }
        }
    }
}
