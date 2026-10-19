use crate::{Feedback, KeyFeedback};
use anyhow::Result;
use eframe::egui::{self, Align, Color32, Layout, Ui, ViewportCommand, vec2};
use std::{
    collections::VecDeque,
    sync::{Arc, OnceLock, mpsc},
    thread,
    time::{Duration, Instant},
};

mod config;
mod render;

pub use config::OverlayConfig;

const WINDOW_TITLE: &str = "keyrun overlay";

// Layout (logical pixels)
const MAX_ROWS: usize = 5;
const ROW_WIDTH: f32 = 240.0;
const ROW_HEIGHT: f32 = 64.0;
const ROW_GAP: f32 = 8.0;
const ROW_STRIDE: f32 = ROW_HEIGHT + ROW_GAP;
const HORIZONTAL_PADDING: f32 = 16.0;
const TOP_PADDING: f32 = 20.0;
const BOTTOM_PADDING: f32 = 10.0;
const WINDOW_WIDTH: f32 = ROW_WIDTH + HORIZONTAL_PADDING * 2.0;
const WINDOW_HEIGHT: f32 = MAX_ROWS as f32 * ROW_STRIDE + TOP_PADDING + BOTTOM_PADDING;

// Animation timings (seconds) and movement (logical pixels)
const ROW_LIFETIME: f32 = 1.2;
const FADE_DURATION: f32 = 0.2;
const CLOSE_HOLD_DURATION: f32 = 0.35;
const SHAKE_DURATION: f32 = 0.3;
const SHAKE_ANGULAR_SPEED: f32 = 90.0;
const SHAKE_DISTANCE: f32 = 6.0;
const PULSE_DURATION: f32 = 0.25;
const PULSE_EXPANSION: f32 = 2.0;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

// Appearance
const ROW_BACKGROUND: Color32 = Color32::from_rgb(24, 27, 33);
const VALID_COLOR: Color32 = Color32::from_rgb(93, 221, 145);
const INVALID_COLOR: Color32 = Color32::from_rgb(255, 105, 115);
const BORDER_WIDTH: f32 = 1.5;
const CORNER_RADIUS: f32 = 9.0;
const FONT_SIZE: f32 = 19.0;
const COMMAND_FONT_SIZE: f32 = 10.0;
const COMMAND_COLOR: Color32 = Color32::from_rgb(210, 213, 220);
const TEXT_PADDING: f32 = 12.0;
const COMMAND_BOTTOM_PADDING: f32 = 9.0;
const TEXT_GAP: f32 = 4.0;

struct Row {
    key: evdev::KeyCode,
    count: usize,
    command: Option<String>,
    label: String,
    valid: bool,
    born: Instant,
}

struct Overlay {
    rx: mpsc::Receiver<KeyFeedback>,
    rows: VecDeque<Row>,
    remaining: Option<usize>,
    closing: Option<Instant>,
    config: OverlayConfig,
}

pub fn run(
    config: OverlayConfig,
    listen: impl FnOnce(Option<Feedback>) -> Result<()> + Send + 'static,
) -> Result<()> {
    config.validate()?;

    let (tx, rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let context = Arc::new(OnceLock::<egui::Context>::new());
    let listener_context = context.clone();

    thread::spawn(move || {
        let feedback = Feedback {
            tx,
            context: listener_context.clone(),
        };
        let _ = result_tx.send(listen(Some(feedback)));

        if let Some(context) = listener_context.get() {
            context.request_repaint();
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_active(false)
            .with_window_type(egui::X11WindowType::Notification)
            .with_taskbar(false)
            .with_mouse_passthrough(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop),

        ..Default::default()
    };

    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |creation| {
            creation.egui_ctx.all_styles_mut(|style| {
                style.visuals.panel_fill = Color32::TRANSPARENT;
                style.visuals.window_fill = Color32::TRANSPARENT;
            });

            let _ = context.set(creation.egui_ctx.clone());

            Ok(Box::new(Overlay {
                rx,
                rows: VecDeque::new(),
                remaining: None,
                closing: None,
                config,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Overlay failed: {error}"))?;

    result_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("Keyboard listener stopped unexpectedly"))?
}

impl Overlay {
    fn receive(&mut self, now: Instant) {
        loop {
            match self.rx.try_recv() {
                Ok(KeyFeedback {
                    key,
                    valid,
                    command,
                    remaining,
                }) => {
                    if valid {
                        self.remaining = remaining;
                    }

                    if valid
                        && let Some(last) = self.rows.back_mut()
                        && last.valid
                        && last.key == key
                        && last.command == command
                        && now.duration_since(last.born).as_secs_f32() < ROW_LIFETIME
                    {
                        last.count = last.count.saturating_add(1);
                        last.born = now;

                        continue;
                    }

                    let name = format!("{key:?}");

                    self.rows.push_back(Row {
                        key,
                        count: 1,
                        command,
                        label: name.trim_start_matches("KEY_").replace('_', " "),
                        valid,
                        born: now,
                    });

                    if self.rows.len() > MAX_ROWS {
                        self.rows.pop_front();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.closing.get_or_insert(now);

                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
    }
}

impl eframe::App for Overlay {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let now = Instant::now();

        self.receive(now);
        self.rows
            .retain(|row| now.duration_since(row.born).as_secs_f32() < ROW_LIFETIME);

        if self.closing.is_some() && self.closing_opacity(now) == 0.0 {
            ctx.send_viewport_cmd(ViewportCommand::Close);

            return;
        }

        // Do not interrupt the listener while a forwarded shortcut is held
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }

        if !self.rows.is_empty() || self.closing.is_some() {
            ctx.request_repaint_after(FRAME_INTERVAL);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let now = Instant::now();
        let opacity = self.closing_opacity(now);

        ui.painter().rect_filled(
            ui.max_rect(),
            CORNER_RADIUS,
            ROW_BACKGROUND.gamma_multiply(self.config.window_background_opacity * opacity),
        );

        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            ui.add_space(BOTTOM_PADDING + ROW_GAP);

            // Iterate newest first so it sits at the bottom of the list
            for row in self.rows.iter().rev() {
                self.show_row(ui, row, now, opacity);
            }
        });
    }
}
