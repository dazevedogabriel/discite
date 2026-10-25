//! Estatísticas (Estatisticas.dc.html): indicadores, revisões por dia,
//! previsão, calibração, contagem de cartões, botões, horário e as
//! distribuições de D, S e R.

use super::charts;
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::tr;
use crate::srs;
use crate::stats::{self, CardCounts, Overview, Scope};
use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

pub struct StatsState {
    pub period: usize,
    /// Escopo escolhido na tela global (None = todas as disciplinas).
    pub subject: Option<i64>,
    cache: Option<(u64, (Option<i64>, usize), Data)>,
}

impl Default for StatsState {
    fn default() -> Self {
        StatsState { period: 1, subject: None, cache: None }
    }
}

#[derive(Default, Clone)]
struct Data {
    ov: Overview,
    retained: (f32, f32),
    daily: Vec<[u32; 3]>,
    forecast: Vec<u32>,
    calib: Vec<(f32, f32, u32)>,
    counts: CardCounts,
    buttons: [[u32; 4]; 3],
    hours: [(u32, u32); 24],
    dist: ([u32; 10], [u32; 10], [u32; 10]),
    desired: f32,
}

fn days_of(period: usize) -> i64 {
    match period {
        0 => 7,
        1 => 30,
        _ => 365,
    }
}

fn load(app: &App, subject: Option<i64>, period: usize) -> Data {
    let decks = subject.map(|s| app.subject_decks(s));
    let scope = Scope(decks.as_deref());
    let days = days_of(period);
    let today = app.clock.today();
    let since = app.clock.day_start(today - days + 1);
    let decay = |d: i64| app.decay_of_deck(d);
    let desired = match subject {
        Some(s) => app.root_decks(s).first().map(|d| app.preset_of_deck(d.id).cfg.desired_retention).unwrap_or(0.9),
        None => app.presets.first().map(|p| p.cfg.desired_retention).unwrap_or(0.9),
    };
    Data {
        ov: stats::overview(&app.db, &scope, since, days as u32).unwrap_or_default(),
        retained: stats::retained(&app.db, &scope, &decay, 30).unwrap_or_default(),
        daily: stats::daily_by_kind(&app.db, &scope, &app.clock, days.min(90)).unwrap_or_default(),
        forecast: stats::forecast(&app.db, &scope, &app.clock, 30).unwrap_or_default(),
        calib: stats::calibration(&app.db, &scope, since).unwrap_or_default(),
        counts: stats::card_counts(&app.db, &scope).unwrap_or_default(),
        buttons: stats::buttons(&app.db, &scope, since).unwrap_or_default(),
        hours: stats::hours(&app.db, &scope, since).unwrap_or([(0, 0); 24]),
        dist: stats::distributions(&app.db, &scope, &decay).unwrap_or_default(),
        desired,
    }
}

/// `fixed_subject`: aba Estatísticas de uma disciplina (sem cabeçalho próprio).
pub fn ui(app: &mut App, ui: &mut Ui, fixed_subject: Option<i64>) {
    let p = pal();
    let subject = fixed_subject.or(app.stats.subject);
    let key = (subject, app.stats.period);
    if app.stats.cache.as_ref().is_none_or(|(v, k, _)| *v != app.data_version || *k != key) {
        let data = load(app, subject, app.stats.period);
        app.stats.cache = Some((app.data_version, key, data));
    }
    let data = app.stats.cache.as_ref().map(|c| c.2.clone()).unwrap_or_default();
    let period_label = tr!(["stats.last_7", "stats.last_30", "stats.last_year"][app.stats.period.min(2)]);
    let scope_label = subject.and_then(|s| app.subject(s)).map(|s| s.name.clone()).unwrap_or_else(|| tr!("review.all_subjects"));
    let periods = [tr!("stats.p7"), tr!("stats.p30"), tr!("stats.p365")];
    let periods: Vec<&str> = periods.iter().map(String::as_str).collect();
    if fixed_subject.is_none() {
        let mut period = app.stats.period;
        let mut pick: Option<Option<i64>> = None;
        let subjects = app.subjects.clone();
        super::header(ui, &tr!("nav.stats"), &format!("{scope_label} · {period_label}"), |ui| {
            segmented(ui, &periods, &mut period, 30.0, false);
            let cur = subject.and_then(|s| subjects.iter().find(|x| x.id == s)).map(|s| s.name.clone()).unwrap_or_else(|| tr!("stats.global"));
            let dot = subject.and_then(|s| subjects.iter().find(|x| x.id == s)).map(|s| subject_color(s.color));
            select_field(ui, "stats_scope", &cur, dot, 190.0, 36.0, |ui| {
                if menu_item(ui, &tr!("stats.global"), subject.is_none()).clicked() {
                    pick = Some(None);
                }
                for s in &subjects {
                    if menu_item_ex(ui, &s.name, Some(subject_color(s.color)), subject == Some(s.id), None).clicked() {
                        pick = Some(Some(s.id));
                    }
                }
            });
        });
        app.stats.period = period;
        if let Some(s) = pick {
            app.stats.subject = s;
        }
    } else {
        // Seletor de período na aba da disciplina.
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
        let mut su = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r.min + vec2(28.0, 12.0), r.max - vec2(28.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        label(&mut su, &format!("{scope_label} · {period_label}"), serif(13.0, W::Regular), p.muted);
        su.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            segmented(ui, &periods, &mut app.stats.period, 28.0, false);
        });
    }

    let area = ui.available_rect_before_wrap();
    let content = Rect::from_min_max(area.min + vec2(28.0, 16.0), area.max - vec2(28.0, 18.0));
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("stats", fixed_subject)).auto_shrink([false, false]).show(&mut cu, |ui| {
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 14.0;
        kpis(ui, &data, w);
        let grid_h = ((content.height() - 70.0 - 28.0) / 2.0).clamp(200.0, 300.0);
        let gap = 14.0;
        let cw = (w - 2.0 * gap) / 3.0;
        let (g, _) = ui.allocate_exact_size(vec2(w, grid_h * 2.0 + gap), Sense::hover());
        let cell = |i: usize| Rect::from_min_size(pos2(g.left() + (i % 3) as f32 * (cw + gap), g.top() + (i / 3) as f32 * (grid_h + gap)), vec2(cw, grid_h));
        daily_card(ui, cell(0), &data);
        forecast_card(ui, cell(1), &data);
        calibration_card(ui, cell(2), &data);
        counts_card(ui, cell(3), &data);
        buttons_card(ui, cell(4), &data);
        hours_card(ui, cell(5), &data);
        let (d, _) = ui.allocate_exact_size(vec2(w, 200.0), Sense::hover());
        dist_card(ui, d, &data);
    });
}

fn kpis(ui: &mut Ui, d: &Data, w: f32) {
    let p = pal();
    let (row, _) = ui.allocate_exact_size(vec2(w, 70.0), Sense::hover());
    let gap = 12.0;
    let cw = (w - 3.0 * gap) / 4.0;
    let h = d.ov.time_ms / 3_600_000;
    let m = (d.ov.time_ms / 60_000) % 60;
    let time = if h > 0 { tr!("stats.time_h", h = h, m = format!("{m:02}")) } else { tr!("time.minutes", n = m) };
    let per_day = d.ov.reviews as f32 / d.ov.days.max(1) as f32;
    let delta = d.retained.0 - d.retained.1;
    let items = [
        (tr!("stats.reviews"), thousands(d.ov.reviews as i64), tr!("stats.per_day", n = srs::decimal(per_day as f64, 1)), p.muted),
        (tr!("stats.true_retention"), d.ov.retention.map(|r| pct(r, 1)).unwrap_or_else(|| "—".into()), tr!("stats.target", pct = pct(d.desired, 0)), p.muted),
        (tr!("stats.time_studied"), time, tr!("stats.per_card", n = srs::decimal(d.ov.avg_secs as f64, 1)), p.muted),
        (tr!("stats.retained"), thousands(d.retained.0.round() as i64), tr!("stats.this_month", delta = format!("{}{}", if delta >= 0.0 { "+" } else { "" }, delta.round())), if delta >= 0.0 { p.good } else { p.again }),
    ];
    for (i, (lab, val, sub, sc)) in items.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(row.left() + i as f32 * (cw + gap), row.top()), vec2(cw, 70.0));
        paint_card(ui.painter(), r);
        caps(ui.painter(), pos2(r.left() + 16.0, r.top() + 18.0), Align2::LEFT_CENTER, &lab, p.dim);
        let v = text(ui.painter(), pos2(r.left() + 16.0, r.top() + 45.0), Align2::LEFT_CENTER, &val, serif(26.0, W::Semi), p.text, cw * 0.6);
        text(ui.painter(), pos2(v.right() + 8.0, r.top() + 49.0), Align2::LEFT_CENTER, &sub, serif(12.0, W::Regular), sc, cw - v.width() - 32.0);
    }
}

fn card_head(ui: &Ui, r: Rect, title: &str, right: &str) -> Rect {
    let p = pal();
    paint_card(ui.painter(), r);
    let inner = r.shrink2(vec2(16.0, 14.0));
    text(ui.painter(), pos2(inner.left(), inner.top() + 9.0), Align2::LEFT_CENTER, title, serif(15.0, W::Semi), p.text, inner.width() * 0.6);
    text(ui.painter(), pos2(inner.right(), inner.top() + 10.0), Align2::RIGHT_CENTER, right, serif(11.5, W::Regular), p.muted, inner.width() * 0.45);
    Rect::from_min_max(pos2(inner.left(), inner.top() + 28.0), inner.max)
}

fn daily_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let body = card_head(ui, r, &tr!("stats.daily"), &tr!("stats.by_kind"));
    let chart = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - 22.0));
    let data: Vec<Vec<f32>> = d.daily.iter().map(|x| vec![x[0] as f32, x[1] as f32, x[2] as f32]).collect();
    charts::stacked(ui.painter(), chart, &data, &[p.good, p.easy, p.again], 2.0);
    if d.daily.iter().all(|x| x.iter().sum::<u32>() == 0) {
        text(ui.painter(), chart.center(), Align2::CENTER_CENTER, &tr!("stats.no_reviews"), serif(12.0, W::Italic), p.muted, chart.width());
    }
    charts::legend(ui.painter(), pos2(body.left(), body.bottom() - 6.0), &[(tr!("stats.k_review").as_str(), p.good), (tr!("stats.k_learning").as_str(), p.easy), (tr!("stats.k_relearning").as_str(), p.again)], false);
}

fn forecast_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let body = card_head(ui, r, &tr!("stats.forecast"), &tr!("stats.next_30"));
    let chart = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - 22.0));
    let vals: Vec<f32> = d.forecast.iter().map(|v| *v as f32).collect();
    charts::bars(ui.painter(), chart, &vals, p.accent, 2.0, |i| if i < 7 { 1.0 } else { 0.5 });
    let total: u32 = d.forecast.iter().sum();
    text(ui.painter(), pos2(body.left(), body.bottom() - 6.0), Align2::LEFT_CENTER, &tr!("stats.tomorrow", n = d.forecast.get(1).copied().unwrap_or(0)), serif(11.0, W::Regular), p.muted, 120.0);
    text(ui.painter(), pos2(body.right(), body.bottom() - 6.0), Align2::RIGHT_CENTER, &tr!("stats.total", n = thousands(total as i64)), serif(11.0, W::Regular), p.muted, 120.0);
}

fn calibration_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let body = card_head(ui, r, &tr!("stats.calibration"), &tr!("stats.calibration_sub"));
    let plot = Rect::from_min_max(pos2(body.left() + 4.0, body.top() + 4.0), pos2(body.right() - 4.0, body.bottom() - 34.0));
    let painter = ui.painter();
    painter.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, p.border));
    painter.vline(plot.left(), plot.y_range(), Stroke::new(1.0, p.border));
    let map = |x: f32, y: f32| pos2(plot.left() + ((x - 0.5) / 0.5).clamp(0.0, 1.0) * plot.width(), plot.bottom() - ((y - 0.5) / 0.5).clamp(0.0, 1.0) * plot.height());
    painter.extend(egui::Shape::dashed_line(&[map(0.5, 0.5), map(1.0, 1.0)], Stroke::new(1.0, p.dim), 3.0, 4.0));
    let max_n = d.calib.iter().map(|c| c.2).max().unwrap_or(1).max(1) as f32;
    for (pred, obs, n) in &d.calib {
        if *pred < 0.5 {
            continue;
        }
        painter.circle_filled(map(*pred, *obs), 2.5 + 5.0 * (*n as f32 / max_n).sqrt(), p.accent);
    }
    text(painter, pos2(plot.left(), plot.bottom() + 10.0), Align2::LEFT_CENTER, "50%", serif(10.0, W::Regular), p.muted, 40.0);
    text(painter, pos2(plot.right(), plot.bottom() + 10.0), Align2::RIGHT_CENTER, "100%", serif(10.0, W::Regular), p.muted, 40.0);
    let msg = match stats::calibration_rmse(&d.calib) {
        Some(e) if e < 0.04 => tr!("stats.calibrated", pct = pct(e, 1)),
        Some(e) => tr!("stats.optimize_hint", pct = pct(e, 1)),
        None => tr!("stats.not_enough"),
    };
    text(painter, pos2(body.left(), body.bottom() - 6.0), Align2::LEFT_CENTER, &msg, serif(11.5, W::Italic), p.muted, body.width());
}

fn counts_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let c = d.counts;
    let body = card_head(ui, r, &tr!("stats.card_count"), &thousands(c.total() as i64));
    let bar = Rect::from_min_size(body.min + vec2(0.0, 4.0), vec2(body.width(), 14.0));
    charts::segments(ui.painter(), bar, &[(c.new as f32, p.easy), (c.learning as f32, p.hard), (c.young as f32, p.good.gamma_multiply(0.6)), (c.mature as f32, p.good), (c.suspended as f32, p.dim)], 4);
    let items = [(tr!("stats.c_new"), c.new, p.easy), (tr!("stats.c_learning"), c.learning, p.hard), (tr!("stats.c_young"), c.young, p.good.gamma_multiply(0.6)), (tr!("stats.c_mature"), c.mature, p.good), (tr!("stats.c_suspended"), c.suspended, p.dim)];
    let cw = (body.width() - 14.0) / 2.0;
    for (i, (lab, n, col)) in items.into_iter().enumerate() {
        let x = body.left() + (i % 2) as f32 * (cw + 14.0);
        let y = bar.bottom() + 22.0 + (i / 2) as f32 * 24.0;
        ui.painter().circle_filled(pos2(x + 4.0, y), 4.0, col);
        text(ui.painter(), pos2(x + 14.0, y), Align2::LEFT_CENTER, &lab, serif(12.5, W::Regular), p.text2, cw - 60.0);
        text(ui.painter(), pos2(x + cw, y), Align2::RIGHT_CENTER, &thousands(n as i64), serif(12.5, W::Regular), p.muted, 60.0);
    }
    text(ui.painter(), pos2(body.left(), body.bottom() - 6.0), Align2::LEFT_CENTER, &tr!("stats.mature_note"), serif(11.0, W::Italic), p.muted, body.width());
}

fn buttons_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let body = card_head(ui, r, &tr!("stats.buttons"), &tr!("stats.answer_freq"));
    let rows = [(tr!("stats.c_learning"), d.buttons[0]), (tr!("stats.c_young"), d.buttons[1]), (tr!("stats.c_mature"), d.buttons[2])];
    let mut y = body.top() + 4.0;
    for (lab, v) in rows {
        let total: u32 = v.iter().sum();
        let ok = if total > 0 { tr!("stats.correct", pct = pct(1.0 - v[0] as f32 / total as f32, 0)) } else { "—".into() };
        text(ui.painter(), pos2(body.left(), y + 7.0), Align2::LEFT_CENTER, &lab, serif(12.0, W::Regular), p.text2, 120.0);
        text(ui.painter(), pos2(body.right(), y + 7.0), Align2::RIGHT_CENTER, &ok, serif(11.5, W::Regular), p.muted, 120.0);
        let bar = Rect::from_min_size(pos2(body.left(), y + 18.0), vec2(body.width(), 12.0));
        charts::segments(ui.painter(), bar, &[(v[0] as f32, p.again), (v[1] as f32, p.hard), (v[2] as f32, p.good), (v[3] as f32, p.easy)], 3);
        y += 42.0;
    }
    let mut x = body.left();
    for (key, c) in [("rating.again", p.again), ("rating.hard", p.hard), ("rating.good", p.good), ("rating.easy", p.easy)] {
        let r = text(ui.painter(), pos2(x, body.bottom() - 6.0), Align2::LEFT_CENTER, &tr!(key), serif(11.0, W::Regular), c, 60.0);
        x = r.right() + 10.0;
    }
}

fn hours_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let body = card_head(ui, r, &tr!("stats.hours"), &tr!("stats.correct_by_hour"));
    let chart = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - 22.0));
    let hours: Vec<(u32, u32)> = d.hours[6..24].to_vec();
    let rates: Vec<f32> = hours.iter().map(|(n, ok)| if *n > 0 { *ok as f32 / *n as f32 } else { 0.0 }).collect();
    let vals: Vec<f32> = rates.iter().map(|r| ((r - 0.5).max(0.0) * 2.0).max(if *r > 0.0 { 0.03 } else { 0.0 })).collect();
    let best = rates.iter().cloned().fold(0.0f32, f32::max);
    charts::bars(ui.painter(), chart, &vals, p.subjects[2], 3.0, |i| if rates[i] >= best - 0.02 && best > 0.0 { 1.0 } else { 0.55 });
    if hours.iter().all(|h| h.0 == 0) {
        text(ui.painter(), chart.center(), Align2::CENTER_CENTER, &tr!("stats.no_reviews"), serif(12.0, W::Italic), p.muted, chart.width());
    }
    let n = 18.0;
    for (h, lab) in [(0.0, "6 h"), (6.0, "12 h"), (12.0, "18 h"), (17.0, "23 h")] {
        let x = chart.left() + (h + 0.5) / n * chart.width();
        text(ui.painter(), pos2(x, body.bottom() - 6.0), Align2::CENTER_CENTER, lab, serif(11.0, W::Regular), p.muted, 40.0);
    }
}

fn dist_card(ui: &Ui, r: Rect, d: &Data) {
    let p = pal();
    let body = card_head(ui, r, &tr!("stats.distributions"), &tr!("stats.studied_cards"));
    let gap = 24.0;
    let cw = (body.width() - 2.0 * gap) / 3.0;
    let sets: [(String, &[u32; 10], [&str; 2], egui::Color32); 3] = [
        (tr!("stats.d_dist"), &d.dist.0, ["1", "10"], p.hard),
        (tr!("stats.s_dist"), &d.dist.1, ["< 1", "≥ 256"], p.good),
        (tr!("stats.r_dist"), &d.dist.2, ["0%", "100%"], p.accent),
    ];
    for (i, (lab, vals, ends, col)) in sets.into_iter().enumerate() {
        let x = body.left() + i as f32 * (cw + gap);
        text(ui.painter(), pos2(x, body.top() + 6.0), Align2::LEFT_CENTER, &lab, serif(12.0, W::Regular), p.text2, cw);
        let chart = Rect::from_min_max(pos2(x, body.top() + 22.0), pos2(x + cw, body.bottom() - 16.0));
        let v: Vec<f32> = vals.iter().map(|v| *v as f32).collect();
        charts::bars(ui.painter(), chart, &v, col, 3.0, |_| 0.85);
        text(ui.painter(), pos2(x, body.bottom() - 4.0), Align2::LEFT_CENTER, ends[0], serif(10.5, W::Regular), p.muted, 50.0);
        text(ui.painter(), pos2(x + cw, body.bottom() - 4.0), Align2::RIGHT_CENTER, ends[1], serif(10.5, W::Regular), p.muted, 50.0);
    }
    let _ = CornerRadius::ZERO;
}
