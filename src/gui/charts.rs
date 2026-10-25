//! Gráficos simples pintados com o `Painter`: barras, barras empilhadas,
//! mapa de calor, curvas e dispersão.

use super::theme::*;
use eframe::egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// Barras verticais. `opacity(i)` permite destacar a primeira, por exemplo.
pub fn bars(painter: &Painter, rect: Rect, values: &[f32], color: Color32, gap: f32, opacity: impl Fn(usize) -> f32) {
    if values.is_empty() {
        return;
    }
    let max = values.iter().cloned().fold(0.0f32, f32::max).max(1.0);
    let w = ((rect.width() - gap * (values.len() as f32 - 1.0)) / values.len() as f32).max(1.0);
    for (i, v) in values.iter().enumerate() {
        let h = (v / max * rect.height()).max(if *v > 0.0 { 2.0 } else { 0.0 });
        let x = rect.left() + i as f32 * (w + gap);
        let r = Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + w, rect.bottom()));
        painter.rect_filled(r, CornerRadius { nw: 2, ne: 2, sw: 1, se: 1 }, color.gamma_multiply(opacity(i)));
    }
}

/// Barras empilhadas (de baixo para cima, na ordem de `colors`).
pub fn stacked(painter: &Painter, rect: Rect, data: &[Vec<f32>], colors: &[Color32], gap: f32) {
    if data.is_empty() {
        return;
    }
    let max = data.iter().map(|d| d.iter().sum::<f32>()).fold(0.0f32, f32::max).max(1.0);
    let w = ((rect.width() - gap * (data.len() as f32 - 1.0)) / data.len() as f32).max(1.0);
    for (i, d) in data.iter().enumerate() {
        let x = rect.left() + i as f32 * (w + gap);
        let mut y = rect.bottom();
        for (v, c) in d.iter().zip(colors) {
            if *v <= 0.0 {
                continue;
            }
            let h = v / max * rect.height();
            painter.rect_filled(Rect::from_min_max(pos2(x, y - h), pos2(x + w, y)), CornerRadius::same(1), *c);
            y -= h + 1.0;
        }
    }
}

/// Barra horizontal segmentada (contagem de cartões, botões).
pub fn segments(painter: &Painter, rect: Rect, parts: &[(f32, Color32)], radius: u8) {
    let total: f32 = parts.iter().map(|p| p.0).sum();
    if total <= 0.0 {
        painter.rect_filled(rect, CornerRadius::same(radius), pal().chip);
        return;
    }
    let n = parts.iter().filter(|p| p.0 > 0.0).count();
    let gap = 2.0;
    let usable = rect.width() - gap * (n.saturating_sub(1)) as f32;
    let mut x = rect.left();
    let mut first = true;
    let last_idx = parts.iter().rposition(|p| p.0 > 0.0).unwrap_or(0);
    for (i, (v, c)) in parts.iter().enumerate() {
        if *v <= 0.0 {
            continue;
        }
        let w = usable * v / total;
        let r = Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.bottom()));
        let cr = CornerRadius { nw: if first { radius } else { 0 }, sw: if first { radius } else { 0 }, ne: if i == last_idx { radius } else { 0 }, se: if i == last_idx { radius } else { 0 } };
        painter.rect_filled(r, cr, *c);
        x += w + gap;
        first = false;
    }
}

/// Mapa de calor anual: `levels[semana][dia]` de 0 a 4.
pub fn heatmap(painter: &Painter, origin: Pos2, levels: &[[u8; 7]], cell: f32, gap: f32) {
    let p = pal();
    for (w, week) in levels.iter().enumerate() {
        for (d, lvl) in week.iter().enumerate() {
            if *lvl == 255 {
                continue;
            }
            let r = Rect::from_min_size(origin + vec2(w as f32 * (cell + gap), d as f32 * (cell + gap)), vec2(cell, cell));
            painter.rect_filled(r, CornerRadius::same(2), p.heat[(*lvl as usize).min(4)]);
        }
    }
}

/// Curva em `rect` com x e y normalizados (0–1).
pub fn curve(painter: &Painter, rect: Rect, pts: &[(f32, f32)], color: Color32, width: f32, dashed: bool) {
    let map = |(x, y): (f32, f32)| pos2(rect.left() + x * rect.width(), rect.bottom() - y * rect.height());
    let pts: Vec<Pos2> = pts.iter().map(|p| map(*p)).collect();
    if pts.len() < 2 {
        return;
    }
    if dashed {
        painter.extend(Shape::dashed_line(&pts, Stroke::new(width, color), 3.0, 3.0));
    } else {
        painter.add(Shape::line(pts, Stroke::new(width, color)));
    }
}

pub fn dashed_h(painter: &Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    painter.extend(Shape::dashed_line(&[pos2(x0, y), pos2(x1, y)], Stroke::new(1.0, color), 3.0, 4.0));
}

pub fn dashed_v(painter: &Painter, x: f32, y0: f32, y1: f32, color: Color32) {
    painter.extend(Shape::dashed_line(&[pos2(x, y0), pos2(x, y1)], Stroke::new(1.0, color), 3.0, 3.0));
}

/// Legenda "■ rótulo" em linha; devolve a largura usada.
pub fn legend(painter: &Painter, pos: Pos2, items: &[(&str, Color32)], line_style: bool) -> f32 {
    let p = pal();
    let mut x = pos.x;
    for (label, c) in items {
        if line_style {
            painter.hline(x..=x + 14.0, pos.y, Stroke::new(2.0, *c));
            x += 20.0;
        } else {
            painter.rect_filled(Rect::from_center_size(pos2(x + 3.5, pos.y), vec2(7.0, 7.0)), CornerRadius::same(2), *c);
            x += 12.0;
        }
        let r = text(painter, pos2(x, pos.y), Align2::LEFT_CENTER, label, serif(11.0, W::Regular), p.muted, 200.0);
        x = r.right() + 12.0;
    }
    x - pos.x
}
