//! Controles pintados à mão no estilo do design: botões (`.btn`,
//! `.btn-primary`, `.btn-ghost`), teclas (`.kbd`), chips, campos, abas,
//! controle segmentado, chave liga/desliga, menus e cards.

use super::theme::*;
use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

pub fn a11y(resp: &Response, role: egui::accesskit::Role, label: &str, selected: Option<bool>) {
    resp.widget_info(|| match selected {
        Some(s) => egui::WidgetInfo::selected(role, true, s, label),
        None => egui::WidgetInfo::labeled(role, true, label),
    });
}

/// Largura natural de um botão com rótulo e tecla opcional.
pub fn button_width(ui: &Ui, label: &str, kbd_label: Option<&str>, size: f32, pad: f32) -> f32 {
    let p = ui.painter();
    let mut w = text_width(p, label, serif(size, W::Semi)) + 2.0 * pad;
    if let Some(k) = kbd_label {
        w += 8.0 + kbd_width(p, k);
    }
    w
}

pub fn button(ui: &mut Ui, label: &str, kind: Btn, height: f32) -> Response {
    button_kbd(ui, label, None, kind, height)
}

pub fn button_kbd(ui: &mut Ui, label: &str, kbd_label: Option<&str>, kind: Btn, height: f32) -> Response {
    let size = if height >= 44.0 { 15.0 } else if height <= 30.0 { 12.5 } else { 13.0 };
    let pad = if height <= 30.0 { 10.0 } else { 14.0 };
    let w = button_width(ui, label, kbd_label, size, pad);
    button_sized(ui, label, kbd_label, kind, vec2(w, height), true)
}

pub fn button_sized(ui: &mut Ui, label: &str, kbd_label: Option<&str>, kind: Btn, size: Vec2, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    paint_button(ui, rect, &resp, label, kbd_label, kind, enabled);
    a11y(&resp, egui::accesskit::Role::Button, label, None);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

pub fn paint_button(ui: &Ui, rect: Rect, resp: &Response, label: &str, kbd_label: Option<&str>, kind: Btn, enabled: bool) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = pal();
    let hovered = resp.hovered() && enabled;
    let radius = CornerRadius::same(if rect.height() >= 44.0 { 10 } else { 8 });
    let (fill, stroke, color, weight) = match kind {
        Btn::Primary => (if hovered { p.accent_hover } else { p.accent }, Stroke::NONE, p.on_accent, W::Semi),
        Btn::Secondary => (if hovered { p.selected } else { p.chip }, Stroke::new(1.0, p.border_control), if hovered { p.text } else { p.text2 }, W::Regular),
        Btn::Ghost => (if hovered { p.hover } else { Color32::TRANSPARENT }, Stroke::NONE, if hovered { p.text } else { p.muted }, W::Regular),
        Btn::Danger => (if hovered { soft(p.again, 0.12) } else { Color32::TRANSPARENT }, Stroke::new(1.0, soft(p.again, 0.45)), p.again, W::Regular),
    };
    let alpha = if enabled { 1.0 } else { 0.45 };
    let painter = ui.painter();
    painter.rect(rect, radius, fill.gamma_multiply(alpha), Stroke::new(stroke.width, stroke.color.gamma_multiply(alpha)), StrokeKind::Inside);
    let size = if rect.height() >= 44.0 { 15.0 } else if rect.height() <= 30.0 { 12.5 } else { 13.0 };
    let font = serif(size, weight);
    let g = line(painter, label, font, color.gamma_multiply(alpha), rect.width() - 12.0, 0.0);
    let kw = kbd_label.map(|k| 8.0 + kbd_width(painter, k)).unwrap_or(0.0);
    let x = rect.center().x - (g.size().x + kw) / 2.0;
    painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), color);
    if let Some(k) = kbd_label {
        let kpos = pos2(x + g.size().x + 8.0, rect.center().y);
        if kind == Btn::Primary {
            kbd_on_accent(painter, kpos, k);
        } else {
            kbd(painter, kpos, Align2::LEFT_CENTER, k);
        }
    }
}

pub fn kbd_width(painter: &egui::Painter, label: &str) -> f32 {
    text_width(painter, &keys(label), mono(10.5)) + 12.0
}

/// A tecla `.kbd`: mono 10,5, borda com 2 px embaixo.
pub fn kbd(painter: &egui::Painter, pos: egui::Pos2, align: Align2, label: &str) -> Rect {
    let p = pal();
    let w = kbd_width(painter, label);
    let rect = align.anchor_size(pos, vec2(w, 18.0));
    painter.rect(rect, CornerRadius::same(5), p.chip, Stroke::new(1.0, p.border_control), StrokeKind::Inside);
    painter.hline(rect.x_range().shrink(2.0), rect.bottom() - 1.0, Stroke::new(1.0, p.border_control));
    text(painter, rect.center() - vec2(0.0, 0.5), Align2::CENTER_CENTER, label, mono(10.5), p.muted, w);
    rect
}

fn kbd_on_accent(painter: &egui::Painter, pos: egui::Pos2, label: &str) {
    let p = pal();
    let w = kbd_width(painter, label);
    let rect = Align2::LEFT_CENTER.anchor_size(pos, vec2(w, 18.0));
    painter.rect(rect, CornerRadius::same(5), Color32::TRANSPARENT, Stroke::new(1.0, p.on_accent.gamma_multiply(0.7)), StrokeKind::Inside);
    text(painter, rect.center() - vec2(0.0, 0.5), Align2::CENTER_CENTER, label, mono(10.5), p.on_accent.gamma_multiply(0.75), w);
}

/// Tecla com uma legenda ao lado ("Ctrl Z desfazer").
pub fn kbd_hint(ui: &mut Ui, key: &str, label: &str) {
    let p = pal();
    let painter = ui.painter();
    let kw = kbd_width(painter, key);
    let lw = text_width(painter, label, serif(12.0, W::Regular));
    let (rect, _) = ui.allocate_exact_size(vec2(kw + 6.0 + lw, 20.0), Sense::hover());
    kbd(ui.painter(), rect.left_center(), Align2::LEFT_CENTER, key);
    text(ui.painter(), pos2(rect.left() + kw + 6.0, rect.center().y), Align2::LEFT_CENTER, label, serif(12.0, W::Regular), p.muted, lw + 4.0);
}

/// Chip `.chip` (24 px, pílula). `active` pinta borda e texto em destaque.
pub fn chip(ui: &mut Ui, label: &str, active: bool, height: f32) -> Response {
    chip_dot(ui, label, None, active, height)
}

pub fn chip_dot(ui: &mut Ui, label: &str, dot: Option<Color32>, active: bool, height: f32) -> Response {
    let p = pal();
    let size = if height <= 22.0 { 11.5 } else { 12.0 };
    let tw = text_width(ui.painter(), label, serif(size, W::Regular));
    let dw = if dot.is_some() { 12.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(tw + 18.0 + dw, height), Sense::click());
    let hovered = resp.hovered();
    let painter = ui.painter();
    let border = if active { p.accent } else { p.border };
    painter.rect(rect, CornerRadius::same(255), if hovered { p.selected } else { p.chip }, Stroke::new(1.0, border), StrokeKind::Inside);
    if let Some(c) = dot {
        painter.circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.0, c);
    }
    text(painter, pos2(rect.left() + 9.0 + dw, rect.center().y), Align2::LEFT_CENTER, label, serif(size, W::Regular), if active { p.accent } else { p.text2 }, tw + 2.0);
    a11y(&resp, egui::accesskit::Role::Button, label, Some(active));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Card `.card`: fundo, borda e raio 14.
pub fn card_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(14)
}

pub fn paint_card(painter: &egui::Painter, rect: Rect) {
    let p = pal();
    painter.rect(rect, CornerRadius::same(14), p.card, Stroke::new(1.0, p.border), StrokeKind::Inside);
}

/// Controle segmentado (período, algoritmo, lacuna). `accent` pinta o ativo em destaque.
pub fn segmented(ui: &mut Ui, options: &[&str], selected: &mut usize, height: f32, accent: bool) -> bool {
    let p = pal();
    let painter = ui.painter();
    let font = serif(if height <= 26.0 { 12.0 } else { 13.0 }, W::Regular);
    let widths: Vec<f32> = options.iter().map(|o| text_width(painter, o, font.clone()) + 24.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, height + 4.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(8), p.chip, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let mut x = rect.left() + 2.0;
    let mut changed = false;
    for (i, (o, w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(*w, height));
        let resp = ui.interact(r, ui.scope_id().with(("seg", o, i)), Sense::click());
        let on = *selected == i;
        let (bg, fg, wgt) = if on {
            if accent { (p.accent, p.on_accent, W::Semi) } else { (p.selected, p.text, W::Semi) }
        } else if resp.hovered() {
            (p.hover, p.text, W::Regular)
        } else {
            (Color32::TRANSPARENT, p.muted, W::Regular)
        };
        ui.painter().rect_filled(r, CornerRadius::same(6), bg);
        text(ui.painter(), r.center(), Align2::CENTER_CENTER, o, serif(font.size, wgt), fg, *w);
        a11y(&resp, egui::accesskit::Role::Tab, o, Some(on));
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
        x += w;
    }
    changed
}

/// Chave liga/desliga 34×20 do design.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), if *on { p.accent } else { p.border_control });
    let x = if *on { rect.right() - 10.0 } else { rect.left() + 10.0 };
    painter.circle_filled(pos2(x, rect.center().y), 8.0, if *on { p.on_accent } else { p.muted });
    a11y(&resp, egui::accesskit::Role::CheckBox, label, Some(*on));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = pal();
    let tw = if label.is_empty() { 0.0 } else { text_width(ui.painter(), label, serif(13.0, W::Regular)) + 8.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0 + tw, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let b = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(15.0, 15.0));
    paint_check(ui.painter(), b, *on);
    if !label.is_empty() {
        text(ui.painter(), pos2(b.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, serif(13.0, W::Regular), p.text2, tw);
    }
    a11y(&resp, egui::accesskit::Role::CheckBox, label, Some(*on));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn paint_check(painter: &egui::Painter, b: Rect, on: bool) {
    let p = pal();
    if on {
        painter.rect_filled(b, CornerRadius::same(4), p.accent);
        let s = Stroke::new(1.8, p.on_accent);
        painter.line_segment([b.left_center() + vec2(3.5, 0.5), pos2(b.center().x - 1.0, b.bottom() - 4.0)], s);
        painter.line_segment([pos2(b.center().x - 1.0, b.bottom() - 4.0), pos2(b.right() - 3.5, b.top() + 4.0)], s);
    } else {
        painter.rect_stroke(b, CornerRadius::same(4), Stroke::new(1.2, p.border_strong), StrokeKind::Inside);
    }
}

/// Aba `.tab` (40 px, sublinhado em destaque quando ativa).
pub fn tab(ui: &mut Ui, label: &str, on: bool) -> Response {
    let p = pal();
    let font = serif(14.0, if on { W::Semi } else { W::Regular });
    let w = text_width(ui.painter(), label, font.clone()) + 4.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
    let color = if on || resp.hovered() { p.text } else { p.muted };
    text(ui.painter(), rect.center(), Align2::CENTER_CENTER, label, font, color, w + 4.0);
    if on {
        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, p.accent));
    }
    a11y(&resp, egui::accesskit::Role::Tab, label, Some(on));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Campo `.field` que abre um menu (substitui o `<select>`).
pub fn select_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, dot: Option<Color32>, width: f32, height: f32, add: impl FnOnce(&mut Ui)) -> Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter();
    let border = if resp.hovered() { p.border_strong } else { p.border_control };
    painter.rect(rect, CornerRadius::same(8), p.input, Stroke::new(1.0, border), StrokeKind::Inside);
    let mut x = rect.left() + 12.0;
    if let Some(c) = dot {
        painter.circle_filled(pos2(x + 4.0, rect.center().y), 4.0, c);
        x += 16.0;
    }
    text(painter, pos2(x, rect.center().y), Align2::LEFT_CENTER, current, serif(13.0, W::Regular), p.text, rect.right() - x - 26.0);
    text(painter, pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, "▾", serif(11.0, W::Regular), p.dim, 20.0);
    a11y(&resp, egui::accesskit::Role::ComboBox, current, None);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&resp)
        .id(ui.scope_id().with(("select", id)))
        .width(width.max(160.0))
        .frame(popup_frame())
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            egui::ScrollArea::vertical().max_height(360.0).show(ui, add);
        });
    resp
}

pub fn popup_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::NONE
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border_control))
        .corner_radius(10)
        .inner_margin(6)
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }) })
}

/// Linha de menu: rótulo, marcação de selecionado e dica opcional à direita.
pub fn menu_item(ui: &mut Ui, label: &str, selected: bool) -> Response {
    menu_item_ex(ui, label, None, selected, None)
}

pub fn menu_item_ex(ui: &mut Ui, label: &str, dot: Option<Color32>, selected: bool, hint: Option<&str>) -> Response {
    let p = pal();
    let w = ui.available_width().max(140.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), p.hover);
    } else if selected {
        painter.rect_filled(rect, CornerRadius::same(6), p.accent_soft);
    }
    let mut x = rect.left() + 10.0;
    if let Some(c) = dot {
        painter.circle_filled(pos2(x + 4.0, rect.center().y), 4.0, c);
        x += 16.0;
    }
    let hw = hint.map(|h| text_width(painter, h, serif(12.0, W::Regular)) + 12.0).unwrap_or(0.0);
    text(painter, pos2(x, rect.center().y), Align2::LEFT_CENTER, label, serif(13.0, if selected { W::Semi } else { W::Regular }), if selected { p.text } else { p.text2 }, rect.right() - x - hw - 8.0);
    if let Some(h) = hint {
        text(painter, pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, h, serif(12.0, W::Regular), p.muted, hw);
    }
    a11y(&resp, egui::accesskit::Role::MenuItem, label, Some(selected));
    if resp.clicked() {
        ui.close();
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn menu_sep(ui: &mut Ui) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(rect.x_range().shrink(6.0), rect.center().y, Stroke::new(1.0, p.line));
}

/// Campo de texto de uma linha com o visual `.field`.
pub fn text_field(ui: &mut Ui, value: &mut String, hint: &str, width: f32, height: f32) -> egui::Response {
    text_field_font(ui, value, hint, width, height, serif(13.0, W::Regular))
}

pub fn text_field_font(ui: &mut Ui, value: &mut String, hint: &str, width: f32, height: f32, font: egui::FontId) -> egui::Response {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(8), p.input);
    let inner = rect.shrink2(vec2(12.0, 0.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add(
        egui::TextEdit::singleline(value)
            .frame(egui::Frame::NONE)
            .font(font)
            .text_color(p.text)
            .hint_text(egui::RichText::new(hint).color(p.dim).italics())
            .desired_width(inner.width())
            .align(egui::Align2::LEFT_CENTER),
    );
    let focused = resp.has_focus();
    let border = if focused { p.accent } else if resp.hovered() { p.border_strong } else { p.border_control };
    ui.painter().rect(rect, CornerRadius::same(8), Color32::TRANSPARENT, Stroke::new(1.0, border), StrokeKind::Inside);
    if focused {
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(10), Stroke::new(3.0, p.accent_soft), StrokeKind::Inside);
    }
    resp
}

/// Campo numérico com o visual `.field`.
pub fn number_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut u32, suffix: &str, width: f32) -> bool {
    let key = ui.scope_id().with(("numfield", id));
    let mut s: String = ui.data_mut(|d| d.get_temp::<String>(key)).unwrap_or_else(|| value.to_string());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let resp = text_field(&mut child, &mut s, "", width, 36.0);
    if !suffix.is_empty() {
        let p = pal();
        text(ui.painter(), pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, suffix, serif(13.0, W::Regular), p.muted, 80.0);
    }
    let mut changed = false;
    if resp.changed() {
        if let Ok(v) = s.trim().replace('.', "").parse::<u32>() {
            if v != *value {
                *value = v;
                changed = true;
            }
        }
    }
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, s));
    } else {
        ui.data_mut(|d| d.remove::<String>(key));
    }
    changed
}

/// Ícone pequeno quadrado (✕, +, ⋯) sem moldura.
pub fn icon_button(ui: &mut Ui, glyph: &str, label: &str, size: f32) -> Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.hover);
    }
    text(ui.painter(), rect.center(), Align2::CENTER_CENTER, glyph, serif(if size >= 28.0 { 16.0 } else { 14.0 }, W::Regular), if resp.hovered() { p.text } else { p.muted }, size);
    a11y(&resp, egui::accesskit::Role::Button, label, None);
    resp.on_hover_text(keys(label)).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Barra de progresso fina.
pub fn progress(painter: &egui::Painter, rect: Rect, frac: f32, color: Color32) {
    let p = pal();
    let r = CornerRadius::same((rect.height() / 2.0) as u8);
    painter.rect_filled(rect, r, p.chip);
    let mut f = rect;
    f.set_width(rect.width() * frac.clamp(0.0, 1.0));
    painter.rect_filled(f, r, color);
}

/// Rótulo `.caps` alocado no layout.
pub fn caps_label(ui: &mut Ui, s: &str) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    caps(ui.painter(), rect.left_center(), Align2::LEFT_CENTER, s, p.dim);
}

/// Texto simples alocado (uma linha).
pub fn label(ui: &mut Ui, s: &str, font: egui::FontId, color: Color32) -> Response {
    let w = ui.available_width();
    let g = line(ui.painter(), s, font, color, w, 0.0);
    let (rect, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
    ui.painter().galley(rect.min, g, color);
    resp
}

/// Texto que quebra linha.
pub fn para(ui: &mut Ui, s: &str, font: egui::FontId, color: Color32) -> Response {
    let w = ui.available_width();
    let g = ui.painter().layout(keys(s).into_owned(), font, color, w);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
    ui.painter().galley(rect.min, g, color);
    resp
}
