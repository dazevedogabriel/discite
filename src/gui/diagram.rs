//! Bloco de fluxograma / mapa mental: caixas que se criam com clique duplo,
//! arrastam e se ligam puxando o ● da borda (soltar no vazio cria uma caixa
//! já ligada). Seleção: clique, Ctrl+clique soma/tira, arrastar no fundo faz
//! um retângulo (com Ctrl, soma), Ctrl+A seleciona tudo. Pan: arrastar o fundo
//! com o botão do meio ou o direito, ou com Espaço apertado. Com uma caixa
//! selecionada: Tab cria uma filha, Enter uma irmã, F2 edita. Delete apaga a
//! seleção; clique direito muda formato e cor.

use eframe::egui::{self, Align2, CornerRadius, Id, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::theme::*;
use super::widgets::{a11y, icon_button, menu_item, menu_item_ex, menu_sep};
use crate::db::{Diagram, DiagramEdge, DiagramNode};
use crate::rich::Ink;
use crate::tr;

const WRAP: f32 = 190.0;
const MIN_H: f32 = 260.0;
/// Acima disso o quadro não cresce: o resto se alcança com o pan.
const MAX_H: f32 = 560.0;

/// Estado da interação, só em memória.
#[derive(Clone, Default)]
struct View {
    nodes: Vec<u32>,
    edge: Option<usize>,
    editing: Option<u32>,
    focus_text: bool,
    link_from: Option<u32>,
    /// Começo do retângulo de seleção (na tela).
    band: Option<Pos2>,
}

impl View {
    fn only(&mut self, id: u32) {
        self.nodes = vec![id];
        self.edge = None;
    }
    fn single(&self) -> Option<u32> {
        (self.nodes.len() == 1).then(|| self.nodes[0])
    }
}

fn node_rect(painter: &egui::Painter, n: &DiagramNode) -> Rect {
    let shown = if n.text.trim().is_empty() { " " } else { n.text.as_str() };
    let g = painter.layout(shown.to_owned(), serif(14.0, W::Regular), egui::Color32::WHITE, WRAP);
    let mut size = vec2((g.size().x + 30.0).max(96.0), (g.size().y + 20.0).max(40.0));
    if n.shape == "diamond" {
        size *= 1.45;
    }
    Rect::from_center_size(pos2(n.x, n.y), size)
}

/// Ponto da borda da caixa na direção de `toward` (para a seta encostar na borda).
fn border(r: Rect, shape: &str, toward: Pos2) -> Pos2 {
    let c = r.center();
    let d = toward - c;
    if d.length() < 1.0 {
        return c;
    }
    let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
    let t = match shape {
        "diamond" => 1.0 / (d.x.abs() / hw + d.y.abs() / hh),
        "pill" => 1.0 / ((d.x / hw).powi(2) + (d.y / hh).powi(2)).sqrt(),
        _ => (hw / d.x.abs().max(1e-3)).min(hh / d.y.abs().max(1e-3)),
    };
    c + d * t.min(1.0)
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-3)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

fn arrow(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    painter.line_segment([a, b], stroke);
    let dir = (b - a).normalized();
    let n = vec2(-dir.y, dir.x);
    let tip = [b, b - dir * 10.0 + n * 5.0, b - dir * 10.0 - n * 5.0];
    painter.add(egui::Shape::convex_polygon(tip.to_vec(), stroke.color, Stroke::NONE));
}

fn next_id(d: &Diagram) -> u32 {
    d.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1
}

fn add_node(d: &mut Diagram, x: f32, y: f32) -> u32 {
    let id = next_id(d);
    d.nodes.push(DiagramNode { id, x, y, ..Default::default() });
    id
}

/// Apaga caixas (com as ligações delas).
fn remove_nodes(d: &mut Diagram, ids: &[u32]) {
    d.nodes.retain(|n| !ids.contains(&n.id));
    d.edges.retain(|e| !ids.contains(&e.from) && !ids.contains(&e.to));
}

fn link(d: &mut Diagram, from: u32, to: u32) {
    if from != to && !d.edges.iter().any(|e| (e.from, e.to) == (from, to)) {
        d.edges.push(DiagramEdge { from, to });
    }
}

/// Caixas que o retângulo de seleção (em coordenadas do diagrama) toca.
fn in_band(rects: &[(u32, Rect)], band: Rect) -> Vec<u32> {
    rects.iter().filter(|(_, r)| r.intersects(band)).map(|(id, _)| *id).collect()
}

/// Pan que mostra todo o conteúdo: centrado se couber, senão a partir do canto.
fn fit_pan(content: Rect, view: Vec2) -> Vec2 {
    let axis = |lo: f32, size: f32, room: f32| if size + 40.0 <= room { (room - size) / 2.0 - lo } else { 20.0 - lo };
    vec2(axis(content.left(), content.width(), view.x), axis(content.top(), content.height(), view.y))
}

/// Desenha e edita o diagrama. Devolve se algo mudou.
pub fn show(ui: &mut Ui, block_id: i64, d: &mut Diagram, w: f32) -> bool {
    let p = pal();
    let vid = Id::unique(("diagram_view", block_id));
    let mut v: View = ui.data(|m| m.get_temp(vid)).unwrap_or_default();
    v.nodes.retain(|id| d.nodes.iter().any(|n| n.id == *id));
    if v.edge.is_some_and(|i| i >= d.edges.len()) {
        v.edge = None;
    }
    let mut changed = false;
    let measure = ui.painter().clone();

    // Caixas em coordenadas do diagrama; a tela soma o canto do quadro e o pan.
    let rects: Vec<Rect> = d.nodes.iter().map(|n| node_rect(&measure, n)).collect();
    let content = rects.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
    let h = if content.is_positive() { (content.height() + 120.0).clamp(MIN_H, MAX_H) } else { MIN_H };
    let (canvas, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let cid = Id::unique(("diagram", block_id));
    let bg = ui.interact(canvas, cid, Sense::click_and_drag());
    let painter = ui.painter_at(canvas);
    let pan = vec2(d.pan_x, d.pan_y);
    let o = canvas.min.to_vec2() + pan;
    let at = |r: Rect| r.translate(o);
    let focused = ui.memory(|m| m.has_focus(cid));
    let (ctrl, space) = ui.input(|i| (i.modifiers.command || i.modifiers.ctrl, i.key_down(Key::Space)));
    let pointer = ui.ctx().pointer_latest_pos();

    painter.rect_filled(canvas, CornerRadius::same(10), p.input);
    // Grade de pontos que acompanha o pan.
    let step = 20.0;
    let start = canvas.min + vec2(pan.x.rem_euclid(step), pan.y.rem_euclid(step));
    let mut y = start.y;
    while y < canvas.bottom() {
        let mut x = start.x;
        while x < canvas.right() {
            painter.circle_filled(pos2(x, y), 0.9, p.line);
            x += step;
        }
        y += step;
    }

    // ── ligações ──
    let mut edge_hover = None;
    for (i, e) in d.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (d.nodes.iter().position(|n| n.id == e.from), d.nodes.iter().position(|n| n.id == e.to)) else { continue };
        let (ra, rb) = (at(rects[a]), at(rects[b]));
        let pa = border(ra, &d.nodes[a].shape, rb.center());
        let pb = border(rb, &d.nodes[b].shape, ra.center());
        let hot = pointer.is_some_and(|q| canvas.contains(q) && dist_to_segment(q, pa, pb) < 6.0);
        if hot {
            edge_hover = Some(i);
        }
        let sel = v.edge == Some(i);
        let col = if sel { p.accent } else if hot { p.text2 } else { p.muted };
        arrow(&painter, pa, pb, Stroke::new(if sel { 2.2 } else { 1.5 }, col));
    }

    // ── caixas ──
    let mut action: Option<(&str, u32, String)> = None;
    let mut moved = Vec2::ZERO;
    for (k, n) in d.nodes.iter().enumerate() {
        let r = at(rects[k]);
        // Fora do quadro: nem desenha nem recebe clique (o bloco de baixo é que recebe).
        if !r.intersects(canvas) {
            continue;
        }
        let resp = ui.interact(r.intersect(canvas), Id::unique(("dnode", block_id, n.id)), Sense::click_and_drag());
        let (fill, edge) = Ink::from_name(&n.color).map_or((p.card, p.border_strong), |k| (soft(ink(k), 0.16), ink(k)));
        let selected = v.nodes.contains(&n.id);
        let stroke = Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { p.accent } else { edge });
        match n.shape.as_str() {
            "diamond" => {
                let pts = vec![pos2(r.center().x, r.top()), pos2(r.right(), r.center().y), pos2(r.center().x, r.bottom()), pos2(r.left(), r.center().y)];
                painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
            }
            "pill" => {
                painter.rect(r, CornerRadius::same((r.height() / 2.0).min(120.0) as u8), fill, stroke, StrokeKind::Inside);
            }
            _ => {
                painter.rect(r, CornerRadius::same(8), fill, stroke, StrokeKind::Inside);
            }
        }
        if v.editing != Some(n.id) {
            let g = painter.layout(n.text.clone(), serif(14.0, W::Regular), p.text, WRAP);
            painter.galley(r.center() - g.size() / 2.0, g, p.text);
        }
        a11y(&resp, egui::accesskit::Role::Button, if n.text.is_empty() { "…" } else { &n.text }, Some(selected));
        if resp.clicked_by(PointerButton::Primary) {
            if ctrl {
                // Ctrl+clique soma ou tira da seleção.
                if selected {
                    v.nodes.retain(|x| *x != n.id);
                } else {
                    v.nodes.push(n.id);
                }
                v.edge = None;
            } else {
                v.only(n.id);
            }
            bg.request_focus();
            if v.editing.is_some_and(|e| e != n.id) {
                v.editing = None;
            }
        }
        if resp.drag_started_by(PointerButton::Primary) {
            // Arrastar uma caixa de fora da seleção começa uma seleção nova.
            if !selected {
                v.only(n.id);
            }
            bg.request_focus();
        }
        if resp.double_clicked() {
            v.only(n.id);
            v.editing = Some(n.id);
            v.focus_text = true;
        }
        if resp.dragged_by(PointerButton::Primary) && v.editing != Some(n.id) {
            moved = resp.drag_delta();
        }
        if resp.drag_stopped() {
            changed = true;
        }
        resp.context_menu(|ui| {
            ui.set_max_width(210.0);
            for (key, shape) in [("diagram.rect", ""), ("diagram.pill", "pill"), ("diagram.diamond", "diamond")] {
                if menu_item(ui, &tr!(key), n.shape == shape).clicked() {
                    action = Some(("shape", n.id, shape.to_owned()));
                }
            }
            menu_sep(ui);
            for c in Ink::ALL.into_iter().map(Some).chain([None]) {
                let name = c.map_or("", |c| c.name());
                let label = c.map_or_else(|| tr!("color.none"), |c| tr!(format!("color.{}", c.name()).as_str()));
                if menu_item_ex(ui, &label, c.map(ink), n.color == name, None).clicked() {
                    action = Some(("color", n.id, name.to_owned()));
                }
            }
            menu_sep(ui);
            if menu_item(ui, &tr!("diagram.edit"), false).clicked() {
                action = Some(("edit", n.id, String::new()));
            }
            if menu_item(ui, &tr!("diagram.delete"), false).clicked() {
                action = Some(("delete", n.id, String::new()));
            }
        });
        // ● na borda direita: puxar liga a outra caixa (ou cria uma nova).
        let show_handle = v.editing.is_none() && (v.single() == Some(n.id) || resp.hovered() || v.link_from == Some(n.id));
        let hp = pos2(r.right(), r.center().y);
        if canvas.contains(hp) {
            let hresp = ui.interact(Rect::from_center_size(hp, vec2(16.0, 16.0)), Id::unique(("dlink", block_id, n.id)), Sense::drag());
            if show_handle || hresp.hovered() {
                painter.circle(hp, if hresp.hovered() { 6.0 } else { 4.5 }, p.accent, Stroke::new(1.5, p.card));
            }
            if hresp.drag_started() {
                v.link_from = Some(n.id);
            }
            if hresp.drag_stopped() && v.link_from == Some(n.id) {
                action = Some(("drop", n.id, String::new()));
            }
            hresp.on_hover_cursor(egui::CursorIcon::Crosshair);
        }
    }
    // Arrastar move a seleção inteira.
    if moved != Vec2::ZERO {
        for n in d.nodes.iter_mut().filter(|n| v.nodes.contains(&n.id)) {
            n.x += moved.x;
            n.y += moved.y;
        }
    }
    // Linha provisória enquanto liga.
    if let (Some(from), Some(q)) = (v.link_from, pointer) {
        if let Some(k) = d.nodes.iter().position(|n| n.id == from) {
            arrow(&painter, border(at(rects[k]), &d.nodes[k].shape, q), q, Stroke::new(1.5, p.accent));
            ui.ctx().request_repaint();
        }
    }
    // Formato, cor e apagar valem para a seleção quando a caixa faz parte dela.
    let targets = |id: u32, v: &View| if v.nodes.contains(&id) { v.nodes.clone() } else { vec![id] };
    match action {
        Some(("drop", from, _)) => {
            v.link_from = None;
            if let Some(q) = pointer {
                let target = d.nodes.iter().enumerate().find(|(k, n)| n.id != from && at(rects[*k]).contains(q)).map(|(_, n)| n.id);
                let to = target.unwrap_or_else(|| {
                    let id = add_node(d, q.x - o.x, q.y - o.y);
                    v.editing = Some(id);
                    v.focus_text = true;
                    id
                });
                link(d, from, to);
                v.only(to);
                changed = true;
            }
        }
        Some((kind @ ("shape" | "color"), id, value)) => {
            let ids = targets(id, &v);
            for n in d.nodes.iter_mut().filter(|n| ids.contains(&n.id)) {
                if kind == "shape" {
                    n.shape = value.clone();
                } else {
                    n.color = value.clone();
                }
            }
            changed = true;
        }
        Some(("edit", id, _)) => {
            v.only(id);
            v.editing = Some(id);
            v.focus_text = true;
        }
        Some(("delete", id, _)) => {
            remove_nodes(d, &targets(id, &v));
            v.nodes.clear();
            changed = true;
        }
        _ => {}
    }

    // ── fundo: pan, retângulo de seleção, clique e clique duplo ──
    let panning = bg.dragged_by(PointerButton::Middle) || bg.dragged_by(PointerButton::Secondary) || (space && bg.dragged_by(PointerButton::Primary));
    if panning {
        d.pan_x += bg.drag_delta().x;
        d.pan_y += bg.drag_delta().y;
        v.band = None;
        changed = true;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if bg.drag_started_by(PointerButton::Primary) {
        v.band = bg.interact_pointer_pos();
    }
    if bg.drag_stopped() {
        if let (Some(a), Some(b)) = (v.band.take(), pointer) {
            let band = Rect::from_two_pos(a, b).translate(-o);
            let ids: Vec<(u32, Rect)> = d.nodes.iter().zip(&rects).map(|(n, r)| (n.id, *r)).collect();
            let hit = in_band(&ids, band);
            if !ctrl {
                v.nodes.clear();
            }
            for id in hit {
                if !v.nodes.contains(&id) {
                    v.nodes.push(id);
                }
            }
            v.edge = None;
            bg.request_focus();
        }
    }
    if let (Some(a), Some(b)) = (v.band, pointer) {
        let r = Rect::from_two_pos(a, b);
        painter.rect(r, CornerRadius::same(3), soft(p.accent, 0.10), Stroke::new(1.0, soft(p.accent, 0.7)), StrokeKind::Inside);
    }
    // Só clique de mouse: Enter/Espaço com o quadro em foco também contam como clique no egui.
    if bg.clicked_by(PointerButton::Primary) {
        if let Some(i) = edge_hover {
            v.nodes.clear();
            v.edge = Some(i);
        } else if !ctrl {
            v.nodes.clear();
            v.edge = None;
        }
        v.editing = None;
        bg.request_focus();
    }
    if bg.double_clicked() && edge_hover.is_none() {
        if let Some(q) = bg.interact_pointer_pos() {
            let id = add_node(d, q.x - o.x, q.y - o.y);
            v.only(id);
            v.editing = Some(id);
            v.focus_text = true;
            changed = true;
        }
    }

    // ── edição do texto da caixa ──
    if let Some(eid) = v.editing {
        if let Some(k) = d.nodes.iter().position(|n| n.id == eid) {
            let r = at(node_rect(&measure, &d.nodes[k]));
            let tid = Id::unique(("dtext", block_id, eid));
            // Enter termina; Shift+Enter quebra a linha.
            let done = ui.memory(|m| m.has_focus(tid)) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Escape));
            let te = egui::TextEdit::multiline(&mut d.nodes[k].text)
                .id(tid)
                .frame(egui::Frame::NONE)
                .font(serif(14.0, W::Regular))
                .text_color(p.text)
                .desired_width(WRAP)
                .desired_rows(1)
                .align(Align2::CENTER_TOP)
                .hint_text(tr!("diagram.hint_text"));
            let resp = ui.put(Rect::from_center_size(r.center(), vec2(WRAP, r.height() - 12.0)).intersect(canvas), te);
            if v.focus_text {
                resp.request_focus();
                v.focus_text = false;
            }
            changed |= resp.changed();
            if done || resp.lost_focus() {
                v.editing = None;
                bg.request_focus();
            }
        } else {
            v.editing = None;
        }
    }

    // ── teclado com o quadro em foco ──
    if focused && v.editing.is_none() {
        ui.memory_mut(|m| m.set_focus_lock_filter(cid, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        let key = |k: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let select_all = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A));
        if select_all {
            v.nodes = d.nodes.iter().map(|n| n.id).collect();
            v.edge = None;
        } else if key(Key::Delete) || key(Key::Backspace) {
            if let Some(i) = v.edge.take() {
                d.edges.remove(i);
                changed = true;
            } else if !v.nodes.is_empty() {
                remove_nodes(d, &v.nodes);
                v.nodes.clear();
                changed = true;
            }
        } else if key(Key::Escape) {
            v.nodes.clear();
            v.edge = None;
        } else if let Some(id) = v.single() {
            let pos = d.nodes.iter().find(|n| n.id == id).map(|n| (n.x, n.y));
            if let Some((x, y)) = pos {
                if key(Key::Tab) {
                    // Filha: à direita, abaixo das irmãs que já existem.
                    let kids = d.edges.iter().filter(|e| e.from == id).count() as f32;
                    let c = add_node(d, x + 240.0, y + kids * 70.0);
                    link(d, id, c);
                    v.only(c);
                    v.editing = Some(c);
                    v.focus_text = true;
                    changed = true;
                } else if key(Key::Enter) {
                    // Irmã: mesma mãe, logo abaixo.
                    let parent = d.edges.iter().find(|e| e.to == id).map(|e| e.from);
                    let c = add_node(d, x, y + 70.0);
                    if let Some(pid) = parent {
                        link(d, pid, c);
                    }
                    v.only(c);
                    v.editing = Some(c);
                    v.focus_text = true;
                    changed = true;
                } else if key(Key::F2) {
                    v.editing = Some(id);
                    v.focus_text = true;
                }
            }
        }
    }

    painter.rect_stroke(canvas, CornerRadius::same(10), Stroke::new(1.0, if focused { p.border_strong } else { p.border }), StrokeKind::Inside);
    if d.nodes.is_empty() {
        text(&painter, canvas.center(), Align2::CENTER_CENTER, &tr!("diagram.empty"), serif(13.0, W::Italic), p.muted, canvas.width() - 40.0);
    } else {
        if focused || !v.nodes.is_empty() {
            text(&painter, pos2(canvas.left() + 12.0, canvas.bottom() - 12.0), Align2::LEFT_CENTER, &tr!("diagram.keys"), serif(11.5, W::Italic), p.dim, canvas.width() - 60.0);
        }
        // Enquadrar: traz todo o conteúdo para a vista.
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(canvas.right() - 36.0, canvas.top() + 6.0), vec2(30.0, 30.0))));
        if icon_button(&mut cu, "⤢", &tr!("diagram.fit"), 28.0).clicked() {
            let fit = fit_pan(content, canvas.size());
            d.pan_x = fit.x;
            d.pan_y = fit.y;
            changed = true;
        }
    }
    ui.data_mut(|m| m.insert_temp(vid, v));
    changed
}

/// O diagrama em SVG (para exportar), com as mesmas caixas, setas e cores da
/// tela no tema claro. `None` se não há caixas.
pub fn svg(painter: &egui::Painter, d: &Diagram) -> Option<String> {
    use super::export::{css, text_svg};
    let p = light();
    let rects: Vec<Rect> = d.nodes.iter().map(|n| node_rect(painter, n)).collect();
    let content = rects.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
    if !content.is_positive() {
        return None;
    }
    let pad = 16.0;
    let shift = vec2(pad, pad) - content.min.to_vec2();
    let (w, h) = (content.width() + 2.0 * pad, content.height() + 2.0 * pad);
    let mut out = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}">"#);
    for e in &d.edges {
        let (Some(a), Some(b)) = (d.nodes.iter().position(|n| n.id == e.from), d.nodes.iter().position(|n| n.id == e.to)) else { continue };
        let (ra, rb) = (rects[a].translate(shift), rects[b].translate(shift));
        let pa = border(ra, &d.nodes[a].shape, rb.center());
        let pb = border(rb, &d.nodes[b].shape, ra.center());
        let dir = (pb - pa).normalized();
        let n = vec2(-dir.y, dir.x);
        let (t1, t2) = (pb - dir * 10.0 + n * 5.0, pb - dir * 10.0 - n * 5.0);
        let c = css(p.muted);
        out.push_str(&format!(r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{c}" stroke-width="1.5"/><polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}" fill="{c}"/>"#, pa.x, pa.y, pb.x, pb.y, pb.x, pb.y, t1.x, t1.y, t2.x, t2.y));
    }
    for (n, r) in d.nodes.iter().zip(&rects) {
        let r = r.translate(shift);
        let (fill, edge) = Ink::from_name(&n.color).map_or((p.card, p.border_strong), |k| (soft(ink_on(k, false), 0.16), ink_on(k, false)));
        let (fill, edge) = (css(fill), css(edge));
        match n.shape.as_str() {
            "diamond" => out.push_str(&format!(
                r#"<polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}" fill="{fill}" stroke="{edge}"/>"#,
                r.center().x, r.top(), r.right(), r.center().y, r.center().x, r.bottom(), r.left(), r.center().y
            )),
            shape => {
                let rx = if shape == "pill" { (r.height() / 2.0).min(120.0) } else { 8.0 };
                out.push_str(&format!(r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{rx:.1}" fill="{fill}" stroke="{edge}"/>"#, r.left() + 0.5, r.top() + 0.5, r.width() - 1.0, r.height() - 1.0));
            }
        }
        let g = painter.layout(n.text.clone(), serif(14.0, W::Regular), p.text, WRAP);
        text_svg(&mut out, &g, r.center() - g.size() / 2.0, p.text);
    }
    out.push_str("</svg>");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_touch_the_border_of_each_shape() {
        let r = Rect::from_center_size(pos2(0.0, 0.0), vec2(100.0, 40.0));
        assert_eq!(border(r, "", pos2(500.0, 0.0)), pos2(50.0, 0.0));
        assert_eq!(border(r, "", pos2(0.0, 500.0)), pos2(0.0, 20.0));
        assert_eq!(border(r, "diamond", pos2(500.0, 0.0)), pos2(50.0, 0.0));
        let q = border(r, "diamond", pos2(100.0, 40.0));
        assert!((q.x / 50.0 + q.y / 20.0 - 1.0).abs() < 1e-4, "{q:?}");
    }

    #[test]
    fn deleting_boxes_drops_their_links_and_plain_reads_the_arrows() {
        let mut d = Diagram::default();
        let a = add_node(&mut d, 100.0, 100.0);
        let b = add_node(&mut d, 300.0, 100.0);
        let c = add_node(&mut d, 300.0, 200.0);
        for (n, t) in d.nodes.iter_mut().zip(["Célula", "Mitose", "Meiose"]) {
            n.text = t.into();
        }
        link(&mut d, a, b);
        link(&mut d, a, c);
        link(&mut d, a, b);
        assert_eq!(d.edges.len(), 2, "ligação repetida não duplica");
        assert_eq!(d.plain(), "Célula → Mitose\nCélula → Meiose");
        remove_nodes(&mut d, &[a]);
        assert!(d.edges.is_empty());
        assert_eq!(d.plain(), "Mitose\nMeiose");
        remove_nodes(&mut d, &[b, c]);
        assert!(d.nodes.is_empty());
    }

    #[test]
    fn band_selects_what_it_touches_and_fit_brings_content_into_view() {
        let r = |x: f32, y: f32| Rect::from_center_size(pos2(x, y), vec2(100.0, 40.0));
        let boxes = [(1, r(100.0, 100.0)), (2, r(300.0, 100.0)), (3, r(100.0, 400.0))];
        assert_eq!(in_band(&boxes, Rect::from_min_max(pos2(0.0, 0.0), pos2(260.0, 200.0))), vec![1, 2]);
        assert!(in_band(&boxes, Rect::from_min_max(pos2(500.0, 500.0), pos2(600.0, 600.0))).is_empty());
        // Cabe: fica centrado.
        let small = Rect::from_min_max(pos2(-500.0, -300.0), pos2(-300.0, -200.0));
        assert_eq!(fit_pan(small, vec2(600.0, 300.0)), vec2(700.0, 400.0));
        // Não cabe: começa pelo canto.
        let big = Rect::from_min_max(pos2(50.0, 0.0), pos2(2050.0, 900.0));
        assert_eq!(fit_pan(big, vec2(600.0, 300.0)), vec2(-30.0, 20.0));
    }
}
