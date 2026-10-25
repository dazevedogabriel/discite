//! Painel (Main.dc.html): "Hoje", cartões das disciplinas, mapa de calor do
//! ano, páginas recentes e previsão dos próximos 7 dias.

use super::charts;
use super::theme::*;
use super::widgets::*;
use super::{App, Screen, review};
use crate::{tr, tr_n};
use crate::db::PageMeta;
use crate::stats::{self, Scope};
use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::collections::HashMap;

#[derive(Default)]
pub struct DashState {
    version: u64,
    day: i64,
    heat: HashMap<i64, u32>,
    forecast: Vec<u32>,
    recent: Vec<PageMeta>,
    /// Disciplina → (retenção 30 d, total de cartões, revisões amanhã).
    subj: HashMap<i64, (Option<f32>, u32, u32)>,
}

fn refresh(app: &mut App) {
    let today = app.clock.today();
    if app.dash.version == app.data_version && app.dash.day == today {
        return;
    }
    let since_year = app.clock.day_start(today - 371);
    let heat = stats::per_day(&app.db, &Scope(None), &app.clock, since_year).unwrap_or_default();
    let forecast = stats::forecast(&app.db, &Scope(None), &app.clock, 8).unwrap_or_default();
    let recent = app.db.recent_pages(5).unwrap_or_default();
    let since30 = app.clock.day_start(today - 29);
    let mut subj = HashMap::new();
    for s in app.subjects.clone() {
        let decks = app.subject_decks(s.id);
        let ov = stats::overview(&app.db, &Scope(Some(&decks)), since30, 30).ok();
        let counts = stats::card_counts(&app.db, &Scope(Some(&decks))).unwrap_or_default();
        let fc = stats::forecast(&app.db, &Scope(Some(&decks)), &app.clock, 2).unwrap_or_default();
        subj.insert(s.id, (ov.and_then(|o| o.retention), counts.total(), fc.get(1).copied().unwrap_or(0)));
    }
    app.dash = DashState { version: app.data_version, day: today, heat, forecast, recent, subj };
}

pub fn ui(app: &mut App, ui: &mut Ui) {
    refresh(app);
    let p = pal();
    let subtitle = tr!("dash.subtitle", date = long_date(), hour = format!("{:02}", app.cfg.rollover_hour));
    let mut open_palette = false;
    let mut open_settings = false;
    super::header(ui, &greeting(), &subtitle, |ui| {
        if icon_button(ui, "⚙", &tr!("common.settings"), 36.0).clicked() {
            open_settings = true;
        }
        // "Buscar ou executar…" abre a paleta.
        let (r, resp) = ui.allocate_exact_size(vec2(300.0, 36.0), Sense::click());
        ui.painter().rect(r, CornerRadius::same(8), p.input, Stroke::new(1.0, if resp.hovered() { p.border_strong } else { p.border_control }), StrokeKind::Inside);
        text(ui.painter(), pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, "⌕", serif(15.0, W::Regular), p.muted, 20.0);
        text(ui.painter(), pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, &tr!("dash.search"), serif(13.0, W::Italic), p.muted, 200.0);
        kbd(ui.painter(), pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, "Ctrl K");
        a11y(&resp, egui::accesskit::Role::Button, &tr!("dash.open_palette"), None);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            open_palette = true;
        }
    });
    if open_palette {
        app.palette = Some(Default::default());
    }
    if open_settings {
        app.go(Screen::Settings);
    }

    let area = ui.available_rect_before_wrap();
    let content = Rect::from_min_max(area.min + vec2(28.0, 22.0), area.max - vec2(28.0, 24.0));
    if app.subjects.is_empty() {
        welcome(app, ui, content);
        return;
    }
    let right_w = 300.0;
    let left = Rect::from_min_max(content.min, pos2(content.right() - right_w - 20.0, content.bottom()));
    let right = Rect::from_min_max(pos2(content.right() - right_w, content.top()), content.max);

    // ── Hoje ──
    let today_rect = Rect::from_min_size(left.min, vec2(left.width(), 150.0));
    ui.painter().rect(today_rect, CornerRadius::same(14), p.card, Stroke::new(1.0, p.border), StrokeKind::Inside);
    ui.painter().rect_filled(today_rect.shrink(1.0), CornerRadius::same(13), p.accent_tint);
    let c = app.total_counts();
    let total = c.new + c.learn + c.review;
    let tl = today_rect.min + vec2(24.0, 22.0);
    caps(ui.painter(), tl + vec2(0.0, 6.0), Align2::LEFT_CENTER, &tr!("dash.today"), p.accent);
    let num = line(ui.painter(), &total.to_string(), serif(56.0, W::Semi), p.text, 200.0, -1.5);
    let num_pos = tl + vec2(0.0, 18.0);
    ui.painter().galley(num_pos, num.clone(), p.text);
    let after = pos2(num_pos.x + num.size().x + 14.0, num_pos.y + num.size().y - 22.0);
    let r = text(ui.painter(), after, Align2::LEFT_CENTER, &tr_n!("dash.cards_to_study", total), serif(17.0, W::Regular), p.text2, 300.0);
    let mins = app.minutes_for(total);
    text(ui.painter(), pos2(r.right() + 6.0, after.y), Align2::LEFT_CENTER, &tr!("dash.about_minutes", n = mins.max(if total > 0 { 1 } else { 0 })), serif(17.0, W::Italic), p.muted, 200.0);
    let mut x = tl.x;
    let y = today_rect.bottom() - 26.0;
    for (n, key, col) in [(c.review, "dash.review", p.good), (c.new, "dash.new", p.easy), (c.learn, "dash.learning", p.hard)] {
        let label = &tr!(key);
        ui.painter().circle_filled(pos2(x + 4.0, y), 4.0, col);
        let r1 = text(ui.painter(), pos2(x + 15.0, y), Align2::LEFT_CENTER, &n.to_string(), serif(13.0, W::Semi), p.text, 60.0);
        let r2 = text(ui.painter(), pos2(r1.right() + 5.0, y), Align2::LEFT_CENTER, label, serif(13.0, W::Regular), p.muted, 120.0);
        x = r2.right() + 22.0;
    }
    let col = Rect::from_min_size(pos2(today_rect.right() - 24.0 - 220.0, today_rect.top() + 22.0), vec2(220.0, 106.0));
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Center)));
    cu.spacing_mut().item_spacing.y = 10.0;
    if button_sized(&mut cu, &tr!("dash.study_all"), Some(&tr!("common.key_space")), Btn::Primary, vec2(220.0, 46.0), total > 0).clicked() {
        review::start(app, review::Scope::All);
    }
    if button_sized(&mut cu, &tr!("dash.custom_session"), None, Btn::Secondary, vec2(220.0, 36.0), true).clicked() {
        app.modal = Some(super::modals::Modal::custom_study(None));
    }
    let avg = tr!("dash.avg_time", n = crate::srs::decimal(app.avg_secs as f64, 1));
    text(cu.painter(), pos2(col.center().x, col.bottom() - 4.0), Align2::CENTER_CENTER, &avg, serif(11.5, W::Italic), p.muted, 240.0);

    // ── Disciplinas ──
    let per_row = 4usize;
    let subjects = app.subjects.clone();
    let rows = subjects.len().div_ceil(per_row);
    let gap = 12.0;
    let cw = (left.width() - gap * (per_row as f32 - 1.0)) / per_row as f32;
    let ch = 112.0;
    let grid_top = today_rect.bottom() + 18.0;
    let visible_rows = rows.min(2);
    for (i, s) in subjects.iter().enumerate().take(visible_rows * per_row) {
        let (row, colx) = (i / per_row, i % per_row);
        let r = Rect::from_min_size(pos2(left.left() + colx as f32 * (cw + gap), grid_top + row as f32 * (ch + gap)), vec2(cw, ch));
        let resp = ui.interact(r, ui.scope_id().with(("subj", s.id)), Sense::click());
        ui.painter().rect(r, CornerRadius::same(14), if resp.hovered() { p.hover } else { p.card }, Stroke::new(1.0, p.border), StrokeKind::Inside);
        let color = subject_color(s.color);
        let x0 = r.left() + 16.0;
        ui.painter().circle_filled(pos2(x0 + 4.0, r.top() + 23.0), 4.0, color);
        text(ui.painter(), pos2(x0 + 16.0, r.top() + 23.0), Align2::LEFT_CENTER, &s.name, serif(14.0, W::Semi), p.text, cw - 50.0);
        let k = app.subject_counts(s.id);
        let due = k.review + k.learn;
        let (ret, total_cards, tomorrow) = app.dash.subj.get(&s.id).copied().unwrap_or((None, 0, 0));
        if due + k.new > 0 {
            let g = line(ui.painter(), &due.to_string(), serif(28.0, W::Semi), p.text, 100.0, 0.0);
            let gp = pos2(x0, r.top() + 38.0);
            ui.painter().galley(gp, g.clone(), p.text);
            let tx = gp.x + g.size().x + 6.0;
            let full = tr!("dash.due_new", n = k.new);
            let label_s = if text_width(ui.painter(), &full, serif(12.0, W::Regular)) < r.right() - 14.0 - tx { full } else { tr!("dash.plus_new", n = k.new) };
            text(ui.painter(), pos2(tx, gp.y + g.size().y - 10.0), Align2::LEFT_CENTER, &label_s, serif(12.0, W::Regular), p.muted, r.right() - 14.0 - tx);
        } else {
            text(ui.painter(), pos2(x0, r.top() + 54.0), Align2::LEFT_CENTER, &tr!(if total_cards == 0 { "dash.no_cards" } else { "dash.all_done" }), serif(15.0, W::Italic), p.text2, cw - 30.0);
        }
        let bar = Rect::from_min_size(pos2(x0, r.top() + 80.0), vec2(cw - 32.0, 4.0));
        progress(ui.painter(), bar, ret.unwrap_or(0.0), color);
        let left_label = if due + k.new == 0 && tomorrow > 0 { tr!("dash.tomorrow", n = tomorrow) } else { ret.map(|v| tr!("dash.retention", pct = pct(v, 0))).unwrap_or_else(|| tr!("dash.retention_none")) };
        let lr = text(ui.painter(), pos2(x0, r.bottom() - 15.0), Align2::LEFT_CENTER, &left_label, serif(11.5, W::Regular), p.muted, cw - 32.0);
        let right_label = tr_n!("dash.card_count", total_cards, count = thousands(total_cards as i64));
        if lr.right() + 10.0 + text_width(ui.painter(), &right_label, serif(11.5, W::Regular)) < r.right() - 16.0 {
            text(ui.painter(), pos2(r.right() - 16.0, r.bottom() - 15.0), Align2::RIGHT_CENTER, &right_label, serif(11.5, W::Regular), p.muted, cw / 2.0);
        }
        a11y(&resp, egui::accesskit::Role::Link, &s.name, None);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.open_subject(s.id);
        }
    }

    // ── Mapa de calor ──
    let heat_top = grid_top + visible_rows as f32 * (ch + gap) + 6.0;
    let heat = Rect::from_min_max(pos2(left.left(), heat_top), left.max);
    if heat.height() > 90.0 {
        heat_card(app, ui, heat);
    }

    // ── Coluna direita ──
    let recent_h: f32 = 66.0 + 5.0 * 52.0;
    let rec = Rect::from_min_size(right.min, vec2(right.width(), recent_h.min(right.height() * 0.62)));
    recent_card(app, ui, rec);
    let fc = Rect::from_min_max(pos2(right.left(), rec.bottom() + 18.0), right.max);
    if fc.height() > 100.0 {
        forecast_card(app, ui, fc);
    }
}

fn welcome(app: &mut App, ui: &mut Ui, content: Rect) {
    let p = pal();
    let r = Rect::from_center_size(content.center() - vec2(0.0, 40.0), vec2(560.0, 280.0));
    paint_card(ui.painter(), r);
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(32.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    cu.spacing_mut().item_spacing.y = 12.0;
    caps_label(&mut cu, &tr!("dash.welcome"));
    let g = line(cu.painter(), &tr!("dash.welcome_title"), serif(34.0, W::Semi), p.text, 400.0, -0.5);
    let (lr, _) = cu.allocate_exact_size(vec2(400.0, 44.0), Sense::hover());
    cu.painter().galley(lr.min, g.clone(), p.text);
    text(cu.painter(), pos2(lr.left() + g.size().x, lr.top() + g.size().y / 2.0), Align2::LEFT_CENTER, &tr!("dash.welcome_title_em"), serif(34.0, W::Italic), p.accent, 300.0);
    para(
        &mut cu,
        &tr!("dash.welcome_text"),
        serif(15.0, W::Regular),
        p.text2,
    );
    cu.add_space(8.0);
    cu.horizontal(|ui| {
        if button(ui, &tr!("dash.create_first_subject"), Btn::Primary, 38.0).clicked() {
            app.modal = Some(super::modals::Modal::Subject { id: None, name: String::new(), color: 0 });
        }
        if button(ui, &tr!("dash.load_sample"), Btn::Secondary, 38.0).clicked() {
            match super::settings::seed_demo(&app.db) {
                Ok(()) => {
                    app.reload();
                    app.toast(tr!("toast.sample_loaded"));
                }
                Err(e) => app.error(e),
            }
        }
    });
}

fn heat_card(app: &App, ui: &mut Ui, r: Rect) {
    let p = pal();
    paint_card(ui.painter(), r);
    let today = app.clock.today();
    let total: u32 = app.dash.heat.values().sum();
    let studied = app.dash.heat.values().filter(|v| **v > 0).count();
    let top = r.top() + 16.0;
    let t = text(ui.painter(), pos2(r.left() + 20.0, top + 9.0), Align2::LEFT_CENTER, &tr!("dash.year_reviews"), serif(16.0, W::Semi), p.text, 260.0);
    text(ui.painter(), pos2(t.right() + 12.0, top + 10.0), Align2::LEFT_CENTER, &tr!("dash.year_summary", reviews = thousands(total as i64), days = studied), serif(13.0, W::Regular), p.muted, 300.0);
    // Legenda menos → mais.
    let lx = r.right() - 20.0;
    let mx = text(ui.painter(), pos2(lx, top + 10.0), Align2::RIGHT_CENTER, &tr!("dash.more"), serif(11.5, W::Regular), p.muted, 40.0);
    let mut x = mx.left() - 6.0 - 5.0 * 14.0;
    for i in 0..5 {
        ui.painter().rect_filled(Rect::from_min_size(pos2(x, top + 5.0), vec2(10.0, 10.0)), CornerRadius::same(2), p.heat[i]);
        x += 14.0;
    }
    text(ui.painter(), pos2(mx.left() - 10.0 - 5.0 * 14.0, top + 10.0), Align2::RIGHT_CENTER, &tr!("dash.less"), serif(11.5, W::Regular), p.muted, 50.0);

    // 53 semanas terminando na semana atual (colunas de domingo a sábado).
    use chrono::Datelike;
    let wd_today = chrono::Local::now().weekday().num_days_from_sunday() as i64;
    let weeks = 53usize;
    let gap = 2.0;
    let avail_w = r.width() - 40.0;
    let cell = ((avail_w - gap * (weeks as f32 - 1.0)) / weeks as f32).clamp(6.0, 13.0);
    let grid_h = 7.0 * cell + 6.0 * gap;
    let max = app.dash.heat.values().copied().max().unwrap_or(1).max(1) as f32;
    let mut levels = vec![[255u8; 7]; weeks];
    let first_day = today - wd_today - (weeks as i64 - 1) * 7;
    for (w, week) in levels.iter_mut().enumerate() {
        for (d, lvl) in week.iter_mut().enumerate() {
            let day = first_day + w as i64 * 7 + d as i64;
            if day > today {
                continue;
            }
            let n = app.dash.heat.get(&day).copied().unwrap_or(0) as f32;
            *lvl = if n <= 0.0 { 0 } else { (1.0 + (n / max * 3.999).floor()).min(4.0) as u8 };
        }
    }
    let grid_w = weeks as f32 * (cell + gap) - gap;
    let origin = pos2(r.left() + 20.0, (r.top() + 64.0).min(r.bottom() - 14.0 - grid_h));
    // Meses.
    let mut last_month = None;
    for w in 0..weeks {
        let day = first_day + w as i64 * 7;
        let date = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(day);
        let m = date.month0();
        if last_month != Some(m) && w < weeks - 2 {
            if last_month.is_some() {
                text(ui.painter(), pos2(origin.x + w as f32 * (cell + gap), origin.y - 10.0), Align2::LEFT_CENTER, &tr!(MONTHS_SHORT[m as usize]), serif(11.0, W::Regular), p.muted, 40.0);
            }
            last_month = Some(m);
        }
    }
    charts::heatmap(ui.painter(), origin, &levels, cell, gap);
    let _ = grid_w;
}

fn recent_card(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = pal();
    paint_card(ui.painter(), r);
    let inner = r.shrink2(vec2(18.0, 16.0));
    text(ui.painter(), pos2(inner.left(), inner.top() + 10.0), Align2::LEFT_CENTER, &tr!("dash.recent_pages"), serif(16.0, W::Semi), p.text, 200.0);
    kbd(ui.painter(), pos2(inner.right(), inner.top() + 10.0), Align2::RIGHT_CENTER, "Ctrl N");
    let mut y = inner.top() + 28.0;
    let recent = app.dash.recent.clone();
    if recent.is_empty() {
        text(ui.painter(), pos2(inner.left(), y + 20.0), Align2::LEFT_CENTER, &tr!("dash.no_pages"), serif(13.0, W::Italic), p.muted, inner.width());
    }
    let n = recent.len();
    for (i, pg) in recent.into_iter().enumerate() {
        if y + 50.0 > r.bottom() {
            break;
        }
        let row = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 50.0));
        let resp = ui.interact(row, ui.scope_id().with(("recent", pg.id)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(row.expand2(vec2(8.0, 0.0)), CornerRadius::same(6), p.hover);
        }
        text(ui.painter(), pos2(row.left(), row.top() + 16.0), Align2::LEFT_CENTER, &pg.title, serif(14.0, W::Regular), p.text, row.width());
        let (sname, color) = app.subject(pg.subject_id).map(|s| (s.name.clone(), subject_color(s.color))).unwrap_or_default();
        ui.painter().circle_filled(pos2(row.left() + 3.0, row.top() + 35.0), 3.0, color);
        text(ui.painter(), pos2(row.left() + 12.0, row.top() + 35.0), Align2::LEFT_CENTER, &format!("{sname} · {}", ago(pg.updated_at)), serif(11.5, W::Regular), p.muted, row.width() - 12.0);
        if i + 1 < n {
            ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, p.line_row));
        }
        a11y(&resp, egui::accesskit::Role::Link, &pg.title, None);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.open_page(pg.id);
        }
        y += 50.0;
    }
}

fn forecast_card(app: &App, ui: &mut Ui, r: Rect) {
    let p = pal();
    paint_card(ui.painter(), r);
    let inner = r.shrink2(vec2(18.0, 16.0));
    text(ui.painter(), pos2(inner.left(), inner.top() + 10.0), Align2::LEFT_CENTER, &tr!("dash.next_7_days"), serif(16.0, W::Semi), p.text, 200.0);
    let fc: Vec<u32> = app.dash.forecast.iter().take(7).copied().collect();
    let avg = if fc.is_empty() { 0.0 } else { fc.iter().sum::<u32>() as f32 / fc.len() as f32 };
    text(ui.painter(), pos2(inner.right(), inner.top() + 10.0), Align2::RIGHT_CENTER, &tr!("dash.avg_per_day", n = avg.round()), serif(12.0, W::Regular), p.muted, 120.0);
    let chart = Rect::from_min_max(pos2(inner.left(), inner.top() + 44.0), pos2(inner.right(), inner.bottom() - 18.0));
    let max = fc.iter().copied().max().unwrap_or(1).max(1) as f32;
    let n = 7;
    let gap = 8.0;
    let w = (chart.width() - gap * (n as f32 - 1.0)) / n as f32;
    use chrono::Datelike;
    let wd = chrono::Local::now().weekday().num_days_from_monday() as usize;
    for i in 0..n {
        let v = fc.get(i).copied().unwrap_or(0);
        let h = (v as f32 / max * (chart.height() - 18.0)).max(if v > 0 { 3.0 } else { 1.0 });
        let x = chart.left() + i as f32 * (w + gap);
        let bar = Rect::from_min_max(pos2(x, chart.bottom() - h), pos2(x + w, chart.bottom()));
        ui.painter().rect_filled(bar, CornerRadius { nw: 4, ne: 4, sw: 2, se: 2 }, p.accent.gamma_multiply(if i == 0 { 1.0 } else { 0.55 }));
        text(ui.painter(), pos2(bar.center().x, bar.top() - 9.0), Align2::CENTER_CENTER, &v.to_string(), serif(11.0, W::Regular), p.muted, w + 10.0);
        let label = &tr!(if i == 0 { "dash.today_short" } else { WEEKDAYS_SHORT[(wd + i) % 7] });
        text(ui.painter(), pos2(bar.center().x, chart.bottom() + 10.0), Align2::CENTER_CENTER, label, serif(11.0, W::Regular), p.muted, w + 10.0);
    }
}
