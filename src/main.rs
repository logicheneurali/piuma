//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.

use std::time::Instant;

use iced::widget::text;
use iced::{Color, Element, Event, Subscription, Task as Command};
use iced::event;
use iced::time;
use iced::font::Font;

use iced_exwlshell::ExWlSettings;
use iced_exwlshell::layershell::timed;
use iced_exwlshell::reexport::{Anchor, Layer, LayerSize};
use iced_exwlshell::settings::{LayerShellSettings, StartMode};
use iced_exwlshell::to_layer_message;

mod sensors;

// ── colours / layout ────────────────────────────────────────────────

const PANEL_HEIGHT: u32 = 32;
const EXCLUSIVE_ZONE: i32 = 32;
const BG_COLOR: Color = Color::from_rgba(0.06, 0.06, 0.08, 0.85);
const TEXT_COLOR: Color = Color::from_rgb(0.88, 0.88, 0.94);
const FONT_NAME: &str = "Noto Sans";

// ── model ───────────────────────────────────────────────────────────

#[derive(Default)]
struct Panel {
    data: String,
}

// ── messages ────────────────────────────────────────────────────────

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {
    Tick(Instant),
    IcedEvent(Event),
}

// ── namespace ───────────────────────────────────────────────────────

fn namespace() -> String {
    String::from("piuma")
}

// ── update ──────────────────────────────────────────────────────────

fn update(panel: &mut Panel, message: Message, _time: Instant) -> Command<Message> {
    match message {
        Message::Tick(_) => {
            panel.data = sensors::gather();
            Command::none()
        }
        Message::IcedEvent(_e) => Command::none(),
        _ => Command::none(),
    }
}

// ── subscription (timer + iced events) ──────────────────────────────

fn subscription(_panel: &Panel) -> Subscription<Message> {
    iced::Subscription::batch([
        // Tick once per second for sensor refresh.
        time::every(std::time::Duration::from_secs(1)).map(|now| Message::Tick(now)),
        // Forward raw iced events (clicks, etc.) so the widget tree
        // can receive them if we add interactive widgets later.
        event::listen().map(Message::IcedEvent),
    ])
}

// ── view ────────────────────────────────────────────────────────────

fn view<'a>(panel: &'a Panel) -> Element<'a, Message> {
    text(&panel.data)
        .color(TEXT_COLOR)
        .size(14)
        .font(Font::with_name(FONT_NAME))
        .into()
}

// ── style (transparent background so panel sits on desktop) ────────

fn style(_panel: &Panel, theme: &iced::Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: BG_COLOR,
        text_color: theme.palette().text,
    }
}

// ── main ────────────────────────────────────────────────────────────

fn main() -> Result<(), iced_exwlshell::Error> {
    // Initialise logging from RUST_LOG env-var (if set), otherwise debug.
    let _ = simple_logger::SimpleLogger::from_env()
        .with_utc_timestamps()
        .init();

    eprintln!("[piuma] Starting …");

    timed(
        Panel::default,
        namespace,
        update,
        subscription,
        view,
    )
    .style(style)
    .wl_settings(ExWlSettings {
        layer_settings: LayerShellSettings {
            size: LayerSize::fill_width(PANEL_HEIGHT),
            exclusive_zone: EXCLUSIVE_ZONE,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            layer: Layer::Top,
            start_mode: StartMode::Active,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
}
