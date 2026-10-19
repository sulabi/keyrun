use super::*;
use eframe::egui::{Direction, FontId, Label, RichText, Sense, Stroke, StrokeKind, UiBuilder};

fn fade_out(elapsed: f32, hold: f32) -> f32 {
    (1.0 - (elapsed - hold) / FADE_DURATION).clamp(0.0, 1.0)
}

impl Overlay {
    pub(super) fn closing_opacity(&self, now: Instant) -> f32 {
        self.closing.map_or(1.0, |time| {
            fade_out(now.duration_since(time).as_secs_f32(), CLOSE_HOLD_DURATION)
        })
    }

    pub(super) fn show_row(&self, ui: &mut Ui, row: &Row, now: Instant, closing_opacity: f32) {
        let age = now.duration_since(row.born).as_secs_f32();
        let opacity = fade_out(age, ROW_LIFETIME - FADE_DURATION) * closing_opacity;
        let color = if row.valid {
            VALID_COLOR
        } else {
            INVALID_COLOR
        };
        let command = row.command.as_deref().filter(|_| self.config.show_command);

        let (rect, _) = ui.allocate_exact_size(vec2(ROW_WIDTH, ROW_HEIGHT), Sense::hover());

        let shake = if !row.valid && age < SHAKE_DURATION {
            (age * SHAKE_ANGULAR_SPEED).sin() * SHAKE_DISTANCE * (1.0 - age / SHAKE_DURATION)
        } else {
            0.0
        };

        let pulse = if row.valid {
            (1.0 - age / PULSE_DURATION).max(0.0) * PULSE_EXPANSION
        } else {
            0.0
        };

        let border = if self.config.row_borders {
            Stroke::new(BORDER_WIDTH, color.gamma_multiply(opacity))
        } else {
            Stroke::NONE
        };

        ui.painter().rect(
            rect.translate(vec2(shake, 0.0)).expand(pulse),
            CORNER_RADIUS,
            ROW_BACKGROUND.gamma_multiply(self.config.row_background_opacity * opacity),
            border,
            StrokeKind::Inside,
        );

        let mut content = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(TEXT_PADDING, COMMAND_BOTTOM_PADDING)))
                .layout(Layout::top_down(Align::Center)),
        );
        content.set_opacity(opacity);
        content.spacing_mut().item_spacing.y = TEXT_GAP;

        let key_label = if command.is_none() && self.remaining.is_none() && row.count > 1 {
            format!("[ {} ] x{}", row.label, row.count)
        } else {
            format!("[ {} ]", row.label)
        };

        let key_height = if command.is_some() {
            content.available_height() - COMMAND_FONT_SIZE - TEXT_GAP
        } else {
            content.available_height()
        };

        content.allocate_ui_with_layout(
            vec2(content.available_width(), key_height),
            Layout::centered_and_justified(Direction::TopDown),
            |ui| {
                ui.add(
                    Label::new(
                        RichText::new(key_label)
                            .monospace()
                            .size(FONT_SIZE)
                            .color(color),
                    )
                    .selectable(false)
                    .truncate(),
                );
            },
        );

        if let Some(command) = command {
            content.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    Label::new(self.command_text(row, command))
                        .selectable(false)
                        .truncate(),
                );
            });
        }
    }

    fn command_text(&self, row: &Row, command: &str) -> egui::text::LayoutJob {
        let format = egui::TextFormat {
            font_id: FontId::monospace(COMMAND_FONT_SIZE),
            color: COMMAND_COLOR,
            ..Default::default()
        };
        let mut text = egui::text::LayoutJob::default();

        if let Some(remaining) = self.remaining {
            let countdown_format = egui::TextFormat {
                color: INVALID_COLOR,
                ..format.clone()
            };

            text.append(&format!("(↓{remaining}) "), 0.0, countdown_format);
        } else if row.count > 1 {
            text.append(&format!("(x{}) ", row.count), 0.0, format.clone());
        }

        text.append(command, 0.0, format);

        text
    }
}
