//! Janela "Log de diagnóstico": o log desta sessão e o da anterior (o que
//! importa quando o programa fechou sozinho), para copiar e mandar adiante.

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::tr;
use eframe::egui::{self, Align2, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, pos2, vec2};

pub struct LogView {
    previous: bool,
    text: String,
    stale: bool,
}

pub fn open(app: &mut App) {
    app.log_view = Some(LogView { previous: false, text: String::new(), stale: true });
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut v) = app.log_view.take() else { return };
    let path = if v.previous { crate::diag::previous_path() } else { crate::diag::current_path() };
    if v.stale {
        v.text = crate::diag::read(&path).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| tr!("log.empty"));
        v.stale = false;
    }
    let p = pal();
    let screen = ctx.content_rect();
    let size = vec2(940.0f32.min(screen.width() - 40.0), 640.0f32.min(screen.height() - 70.0));
    let mut close = false;
    egui::Modal::new(egui::Id::unique("log_view"))
        .backdrop_color(p.scrim)
        .frame(egui::Frame::NONE.fill(p.window).corner_radius(16).stroke(Stroke::new(1.0, p.border_control)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            let rect = Rect::from_min_size(ui.max_rect().min, size);
            // Cabeçalho: título, abas e fechar.
            let head = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
            ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
            let t = text(ui.painter(), pos2(head.left() + 26.0, head.center().y), Align2::LEFT_CENTER, &tr!("log.title"), serif(19.0, W::Semi), p.text, 300.0);
            let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(t.right() + 20.0, head.top()), pos2(head.right() - 70.0, head.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let mut sel = usize::from(v.previous);
            let (a, b) = (tr!("log.current"), tr!("log.previous"));
            if segmented(&mut hu, &[a.as_str(), b.as_str()], &mut sel, 26.0, false) {
                v.previous = sel == 1;
                v.stale = true;
            }
            let close_r = Rect::from_center_size(pos2(head.right() - 36.0, head.center().y), vec2(40.0, 28.0));
            let cresp = ui.interact(close_r, ui.scope_id().with("close"), Sense::click());
            if cresp.hovered() {
                ui.painter().rect_filled(close_r, CornerRadius::same(6), p.hover);
            }
            close_icon(ui.painter(), close_r.center(), 13.0, if cresp.hovered() { p.text } else { p.muted });
            a11y(&cresp, egui::accesskit::Role::Button, &tr!("app.close_dialog"), None);
            if cresp.clicked() {
                close = true;
            }

            // Rodapé: onde está o arquivo e as ações.
            let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 64.0), rect.max);
            ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, p.line));
            let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(24.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let w = fu.available_width() - 360.0;
            let (pr, _) = fu.allocate_exact_size(vec2(w.max(80.0), 20.0), Sense::hover());
            text(fu.painter(), pr.left_center(), Align2::LEFT_CENTER, &path.display().to_string(), mono(11.0), p.dim, pr.width());
            fu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if button(ui, &tr!("log.copy"), Btn::Primary, 36.0).clicked() {
                    ui.ctx().copy_text(v.text.clone());
                    app.toast(tr!("log.copied"));
                }
                if button(ui, &tr!("log.refresh"), Btn::Secondary, 36.0).clicked() {
                    v.stale = true;
                }
                if button(ui, &tr!("settings.open_folder"), Btn::Secondary, 36.0).clicked() {
                    super::settings::open_folder(&crate::diag::dir());
                }
            });

            // O texto, só para ler e selecionar.
            let mid = Rect::from_min_max(pos2(rect.left(), head.bottom()), pos2(rect.right(), foot.top()));
            let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(mid.shrink2(vec2(20.0, 14.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            egui::ScrollArea::both().id_salt(("log_text", v.previous)).auto_shrink([false, false]).stick_to_bottom(true).show(&mut mu, |ui| {
                ui.add(egui::TextEdit::multiline(&mut v.text.as_str()).font(mono(11.5)).text_color(p.text2).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
            });
        });
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        close = true;
    }
    if !close {
        app.log_view = Some(v);
    }
}
