//! Opções do preset (Preset.dc.html): algoritmo, retenção desejada, passos e
//! limites, ordem, otimizador do FSRS e simulador de carga × retenção.
//!
//! Otimização, avaliação e simulação rodam em threads; o resultado volta por
//! um canal e é lido a cada quadro em `poll_jobs`.

use super::modals::Modal;
use super::theme::*;
use super::widgets::*;
use super::{App, Screen};
use crate::{tr, tr_n};
use crate::db::{self, Preset, Rev};
use crate::srs::{self, Algorithm, LeechAction, NewMix, NewOrder, ReviewOrder};
use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use fsrs::{FSRSItem, FSRSReview};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

pub enum JobResult {
    Optimized { preset: i64, params: Vec<f32>, before: Option<(f32, f32)>, after: Option<(f32, f32)> },
    Evaluated { preset: i64, eval: (f32, f32) },
    Simulated { preset: i64, points: Vec<(f32, f32, f32)> },
    Failed(String),
}

pub struct PresetView {
    pub id: i64,
    draft: Preset,
    learn: String,
    relearn: String,
    dirty: bool,
    reviews: i64,
    rx: Option<Receiver<JobResult>>,
    progress: Option<Arc<Mutex<fsrs::CombinedProgressState>>>,
    running: Option<&'static str>,
    eval_now: Option<(f32, f32)>,
    proposal: Option<(Vec<f32>, Option<(f32, f32)>)>,
    sim: Option<Vec<(f32, f32, f32)>>,
    sim_for: Option<(Vec<f32>, f32)>,
}

impl PresetView {
    fn new(app: &App, id: i64) -> Option<PresetView> {
        let draft = app.preset(id)?.clone();
        let decks = decks_of(app, id);
        Some(PresetView {
            id,
            learn: srs::format_steps(&draft.cfg.learning_steps),
            relearn: srs::format_steps(&draft.cfg.relearning_steps),
            draft,
            dirty: false,
            reviews: app.db.revlog_count(&decks).unwrap_or(0),
            rx: None,
            progress: None,
            running: None,
            eval_now: None,
            proposal: None,
            sim: None,
            sim_for: None,
        })
    }
}

fn decks_of(app: &App, preset: i64) -> Vec<i64> {
    app.decks.iter().filter(|d| d.preset_id == preset).map(|d| d.id).collect()
}

/// Revlog → itens de treino do FSRS: um item por revisão de longo prazo,
/// com todo o histórico anterior do cartão (dias inteiros pelo relógio de estudo).
pub fn fsrs_items(histories: &HashMap<i64, Vec<Rev>>, clock: &srs::Clock) -> Vec<FSRSItem> {
    let mut out = Vec::new();
    for revs in histories.values() {
        // Só cartões cujo histórico começa como novo.
        let Some(start) = revs.iter().position(|r| r.state_before == srs::State::New) else { continue };
        let revs = &revs[start..];
        let mut reviews: Vec<FSRSReview> = Vec::with_capacity(revs.len());
        let mut last_day: Option<i64> = None;
        for r in revs {
            let day = clock.day_of(r.at);
            let delta = last_day.map(|d| (day - d).max(0) as u32).unwrap_or(0);
            reviews.push(FSRSReview { rating: r.rating.clamp(1, 4), delta_t: delta });
            last_day = Some(day);
        }
        for i in 1..reviews.len() {
            if reviews[i].delta_t > 0 {
                out.push(FSRSItem { reviews: reviews[..=i].to_vec() });
            }
        }
    }
    out
}

fn start_job(app: &mut App, kind: &'static str) {
    let Some(view) = app.preset_view.as_mut() else { return };
    if view.running.is_some() {
        return;
    }
    let (tx, rx) = channel();
    view.rx = Some(rx);
    view.running = Some(kind);
    let preset = view.draft.clone();
    let decks: Vec<i64> = app.decks.iter().filter(|d| d.preset_id == preset.id).map(|d| d.id).collect();
    let clock = app.clock;
    match kind {
        "optimize" | "evaluate" => {
            let histories = app.db.histories(&decks).unwrap_or_default();
            let progress = fsrs::CombinedProgressState::new_shared();
            view.progress = Some(progress.clone());
            std::thread::spawn(move || {
                let items = fsrs_items(&histories, &clock);
                let res = (|| -> anyhow::Result<JobResult> {
                    if items.is_empty() {
                        anyhow::bail!(crate::tr!("preset.not_enough_reviews"));
                    }
                    let eval = |params: &[f32]| -> Option<(f32, f32)> { fsrs::FSRS::new(params).ok()?.evaluate(items.clone(), |_| true).ok().map(|e| (e.log_loss, e.rmse_bins)) };
                    if kind == "evaluate" {
                        let e = eval(&preset.params).ok_or_else(|| anyhow::anyhow!(crate::tr!("preset.eval_failed")))?;
                        return Ok(JobResult::Evaluated { preset: preset.id, eval: e });
                    }
                    let before = eval(&preset.params);
                    let params = fsrs::compute_parameters(fsrs::ComputeParametersInput {
                        train_set: items.clone(),
                        progress: Some(progress),
                        enable_short_term: true,
                        num_relearning_steps: Some(preset.cfg.relearning_steps.len()),
                        ..Default::default()
                    })
                    .map_err(|e| anyhow::anyhow!("{}", match e {
                        fsrs::FSRSError::NotEnoughData => crate::tr!("preset.not_enough_data"),
                        other => crate::tr!("preset.optimize_failed", error = format!("{other:?}")),
                    }))?;
                    let after = eval(&params);
                    Ok(JobResult::Optimized { preset: preset.id, params, before, after })
                })();
                let _ = tx.send(res.unwrap_or_else(|e| JobResult::Failed(format!("{e:#}"))));
            });
        }
        _ => {
            let n_cards = crate::stats::card_counts(&app.db, &crate::stats::Scope(Some(&decks))).map(|c| c.total()).unwrap_or(0).max(200) as usize;
            let cfg = preset.cfg.clone();
            let params = preset.params.clone();
            view.sim_for = Some((params.clone(), cfg.desired_retention));
            std::thread::spawn(move || {
                let mut points = Vec::new();
                let sim_cfg = fsrs::SimulatorConfig {
                    deck_size: n_cards,
                    learn_span: 365,
                    max_ivl: cfg.max_interval as f32,
                    learn_limit: cfg.new_per_day.max(1) as usize,
                    review_limit: usize::MAX,
                    learning_step_count: cfg.learning_steps.len().min(5),
                    relearning_step_count: cfg.relearning_steps.len().min(5),
                    ..Default::default()
                };
                let w = if params.len() >= 19 { params.clone() } else { fsrs::DEFAULT_PARAMETERS.to_vec() };
                let mut dr = 0.70f32;
                while dr <= 0.991 {
                    if let Ok(r) = fsrs::simulate(&sim_cfg, &w, dr, Some(42), None) {
                        let days = r.review_cnt_per_day.len().max(1) as f32;
                        let per_day = r.review_cnt_per_day.iter().sum::<usize>() as f32 / days;
                        let mem = r.memorized_cnt_per_day.last().copied().unwrap_or(0.0);
                        points.push((dr, per_day, mem));
                    }
                    dr += 0.01;
                }
                let _ = tx.send(if points.is_empty() { JobResult::Failed(crate::tr!("preset.sim_failed")) } else { JobResult::Simulated { preset: preset.id, points } });
            });
        }
    }
}

pub fn poll_jobs(app: &mut App) {
    let Some(view) = app.preset_view.as_mut() else { return };
    let Some(rx) = &view.rx else { return };
    let Ok(res) = rx.try_recv() else { return };
    view.rx = None;
    view.running = None;
    view.progress = None;
    match res {
        JobResult::Optimized { preset, params, before, after } if preset == view.id => {
            view.eval_now = before.or(view.eval_now);
            view.proposal = Some((params, after));
        }
        JobResult::Evaluated { preset, eval } if preset == view.id => view.eval_now = Some(eval),
        JobResult::Simulated { preset, points } if preset == view.id => view.sim = Some(points),
        JobResult::Failed(e) => app.error(e),
        _ => {}
    }
}

pub fn ui(app: &mut App, ui: &mut Ui, id: i64) {
    if app.preset_view.as_ref().is_none_or(|v| v.id != id) {
        match PresetView::new(app, id) {
            Some(v) => app.preset_view = Some(v),
            None => {
                app.go(Screen::Dashboard);
                return;
            }
        }
        start_job(app, "simulate");
    }
    let p = pal();
    let decks = decks_of(app, id);
    let mut subj_names: Vec<String> = decks.iter().filter_map(|d| app.deck(*d)).filter_map(|d| app.subject(d.subject_id)).map(|s| s.name.clone()).collect();
    subj_names.sort();
    subj_names.dedup();
    let subjects = if subj_names.is_empty() { String::new() } else { tr!("preset.of_subjects", list = join_list(&subj_names)) };
    let sub = tr_n!("preset.shared_by", decks.len(), subjects = subjects);
    let mut save = false;
    let mut restore = false;
    let presets = app.presets.clone();
    let mut goto: Option<i64> = None;
    let mut new_preset = false;
    let mut rename = false;
    let mut delete = false;
    let dirty = app.preset_view.as_ref().is_some_and(|v| v.dirty);
    super::header(ui, &tr!("preset.title"), &sub, |ui| {
        let save_label = tr!("common.save");
        let w = button_width(ui, &save_label, None, 13.0, 14.0);
        if button_sized(ui, &save_label, None, Btn::Primary, vec2(w, 34.0), dirty).clicked() {
            save = true;
        }
        if button(ui, &tr!("preset.restore_defaults"), Btn::Secondary, 34.0).clicked() {
            restore = true;
        }
        let name = app.preset(id).map(|p| p.name.clone()).unwrap_or_default();
        select_field(ui, "preset_sel", &name, None, 210.0, 36.0, |ui| {
            for pr in &presets {
                if menu_item(ui, &pr.name, pr.id == id).clicked() && pr.id != id {
                    goto = Some(pr.id);
                }
            }
            menu_sep(ui);
            if menu_item(ui, &tr!("preset.new"), false).clicked() {
                new_preset = true;
            }
            if menu_item(ui, &tr!("common.rename"), false).clicked() {
                rename = true;
            }
            if id != 1 && menu_item(ui, &tr!("preset.delete"), false).clicked() {
                delete = true;
            }
        });
    });
    if let Some(g) = goto {
        app.preset_view = None;
        app.go(Screen::Preset(g));
        return;
    }
    if new_preset {
        app.modal = Some(Modal::NewPreset { name: String::new(), from: id });
    }
    if rename {
        app.modal = Some(Modal::RenamePreset { id, name: app.preset(id).map(|p| p.name.clone()).unwrap_or_default() });
    }
    if delete {
        let _ = app.db.delete_preset(id);
        app.reload();
        app.preset_view = None;
        app.go(Screen::Preset(1));
        return;
    }
    if restore {
        if let Some(v) = app.preset_view.as_mut() {
            v.draft.cfg = srs::PresetConfig::default();
            v.learn = srs::format_steps(&v.draft.cfg.learning_steps);
            v.relearn = srs::format_steps(&v.draft.cfg.relearning_steps);
            v.dirty = true;
        }
    }
    if save {
        save_preset(app);
    }

    let area = ui.available_rect_before_wrap();
    let content = Rect::from_min_max(area.min + vec2(28.0, 20.0), area.max - vec2(28.0, 24.0));
    let right_w = 380.0;
    let left = Rect::from_min_max(content.min, pos2(content.right() - right_w - 20.0, content.bottom()));
    let right = Rect::from_min_max(pos2(content.right() - right_w, content.top()), content.max);
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt("preset_left").auto_shrink([false, false]).show(&mut lu, |ui| {
        ui.spacing_mut().item_spacing.y = 16.0;
        scheduling_card(app, ui);
        steps_card(app, ui);
        order_card(app, ui);
    });
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::top_down(egui::Align::Min)));
    ru.spacing_mut().item_spacing.y = 16.0;
    optimizer_card(app, &mut ru);
    simulator_card(app, &mut ru);
    if app.preset_view.as_ref().is_some_and(|v| v.running.is_some()) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    let _ = p;
}

/// 0.90 / 0,90 (sempre duas casas).
fn ret2(v: f32) -> String {
    crate::i18n::fixed(v as f64, 2)
}

/// "A, B and C" / "A, B e C".
fn join_list(v: &[String]) -> String {
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        _ => tr!("preset.list_and", rest = v[..v.len() - 1].join(", "), last = v[v.len() - 1]),
    }
}

fn save_preset(app: &mut App) {
    let Some(v) = app.preset_view.as_mut() else { return };
    match (srs::parse_steps(&v.learn), srs::parse_steps(&v.relearn)) {
        (Some(l), Some(r)) => {
            v.draft.cfg.learning_steps = l;
            v.draft.cfg.relearning_steps = r;
        }
        _ => {
            app.toast(tr!("toast.bad_steps"));
            return;
        }
    }
    let draft = v.draft.clone();
    v.dirty = false;
    match app.db.save_preset(&draft) {
        Ok(()) => {
            app.reload();
            app.toast(tr!("toast.preset_saved"));
        }
        Err(e) => app.error(e),
    }
}

fn card_box(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    card_frame().inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 14.0;
        add(ui);
    });
}

fn scheduling_card(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let sim = app.preset_view.as_ref().and_then(|v| v.sim.clone());
    let v = app.preset_view.as_mut().unwrap();
    card_box(ui, |ui| {
        ui.horizontal(|ui| {
            label(ui, &tr!("preset.scheduling"), serif(16.0, W::Semi), p.text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut sel = if v.draft.cfg.algorithm == Algorithm::Fsrs { 0 } else { 1 };
                if segmented(ui, &["FSRS-6", "SM-2"], &mut sel, 26.0, true) {
                    v.draft.cfg.algorithm = if sel == 0 { Algorithm::Fsrs } else { Algorithm::Sm2 };
                    v.dirty = true;
                }
            });
        });
        if v.draft.cfg.algorithm == Algorithm::Sm2 {
            para(ui, &tr!("preset.sm2_note"), serif(12.5, W::Italic), p.muted);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (lab, val, lo, hi) in [(tr!("preset.starting_ease"), &mut v.draft.cfg.starting_ease, 1.3f32, 5.0f32), (tr!("preset.easy_bonus"), &mut v.draft.cfg.easy_bonus, 1.0, 3.0), (tr!("preset.hard_multiplier"), &mut v.draft.cfg.hard_multiplier, 1.0, 2.0)] {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 5.0;
                        caps_label(ui, &lab);
                        if ui.add(egui::DragValue::new(val).speed(0.01).range(lo..=hi).fixed_decimals(2)).changed() {
                            v.dirty = true;
                        }
                    });
                }
            });
            return;
        }
        // Retenção desejada: número grande + controle deslizante.
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
        caps(ui.painter(), pos2(row.left(), row.top() + 8.0), Align2::LEFT_CENTER, &tr!("preset.desired_retention"), p.dim);
        text(ui.painter(), pos2(row.left(), row.top() + 42.0), Align2::LEFT_CENTER, &ret2(v.draft.cfg.desired_retention), serif(44.0, W::Semi), p.text, 140.0);
        let track = Rect::from_min_max(pos2(row.left() + 150.0, row.top() + 22.0), pos2(row.right(), row.top() + 40.0));
        let resp = ui.interact(track.expand2(vec2(9.0, 6.0)), ui.scope_id().with("dr_slider"), Sense::click_and_drag());
        let (lo, hi) = (0.70f32, 0.99f32);
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                let f = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
                let val = ((lo + f * (hi - lo)) * 100.0).round() / 100.0;
                if (val - v.draft.cfg.desired_retention).abs() > 1e-4 {
                    v.draft.cfg.desired_retention = val;
                    v.dirty = true;
                }
            }
        }
        if resp.has_focus() {
            let d = ui.ctx().input(|i| (i.key_pressed(egui::Key::ArrowRight) as i32 - i.key_pressed(egui::Key::ArrowLeft) as i32) as f32 * 0.01);
            if d != 0.0 {
                v.draft.cfg.desired_retention = (v.draft.cfg.desired_retention + d).clamp(lo, hi);
                v.dirty = true;
            }
        }
        a11y(&resp, egui::accesskit::Role::Slider, &tr!("preset.desired_retention"), None);
        let f = (v.draft.cfg.desired_retention - lo) / (hi - lo);
        let cy = track.center().y;
        ui.painter().rect_filled(Rect::from_min_max(pos2(track.left(), cy - 2.0), pos2(track.right(), cy + 2.0)), CornerRadius::same(2), p.chip);
        ui.painter().rect_filled(Rect::from_min_max(pos2(track.left(), cy - 2.0), pos2(track.left() + f * track.width(), cy + 2.0)), CornerRadius::same(2), p.accent);
        let kx = track.left() + f * track.width();
        ui.painter().circle(pos2(kx, cy), 9.0, p.window, Stroke::new(2.0, p.accent));
        for t in [0.70f32, 0.80, 0.90, 0.99] {
            let x = track.left() + (t - lo) / (hi - lo) * track.width();
            text(ui.painter(), pos2(x, row.top() + 56.0), Align2::CENTER_CENTER, &ret2(t), serif(11.0, W::Regular), p.muted, 40.0);
        }
        // Carga estimada (simulador) em três pontos.
        let cur = v.draft.cfg.desired_retention;
        let at = |dr: f32| sim.as_ref().and_then(|pts| pts.iter().min_by(|a, b| (a.0 - dr).abs().partial_cmp(&(b.0 - dr).abs()).unwrap()).map(|p| p.1));
        let (r3, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
        let bw = (r3.width() - 20.0) / 3.0;
        let picks = [(cur - 0.05).max(0.70), cur, (cur + 0.05).min(0.99)];
        for (i, dr) in picks.iter().enumerate() {
            let b = Rect::from_min_size(pos2(r3.left() + i as f32 * (bw + 10.0), r3.top()), vec2(bw, 56.0));
            let current = i == 1;
            ui.painter().rect(b, CornerRadius::same(10), if current { p.accent_tint } else { p.window }, Stroke::new(1.0, if current { p.accent } else { p.border }), StrokeKind::Inside);
            let lab = format!("{}{}", tr!("preset.at", value = ret2(*dr)), if current { tr!("preset.current") } else { String::new() });
            text(ui.painter(), pos2(b.left() + 12.0, b.top() + 17.0), Align2::LEFT_CENTER, &lab, serif(12.0, W::Regular), if current { p.accent } else { p.muted }, bw - 20.0);
            let val = at(*dr).map(|n| tr!("preset.per_day", n = n.round())).unwrap_or_else(|| tr!("preset.computing"));
            text(ui.painter(), pos2(b.left() + 12.0, b.top() + 38.0), Align2::LEFT_CENTER, &val, serif(16.0, W::Semi), p.text, bw - 20.0);
        }
    });
}

fn steps_card(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let v = app.preset_view.as_mut().unwrap();
    card_box(ui, |ui| {
        label(ui, &tr!("preset.steps_limits"), serif(16.0, W::Semi), p.text);
        let w = ui.available_width();
        let cw = (w - 28.0) / 3.0;
        egui::Grid::new("steps_grid").num_columns(3).spacing(vec2(14.0, 12.0)).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                caps_label(ui, &tr!("preset.learning_steps"));
                let ok = srs::parse_steps(&v.learn).is_some();
                if text_field_font(ui, &mut v.learn, "1m 10m", cw, 36.0, mono(12.5)).changed() {
                    v.dirty = true;
                }
                if !ok {
                    label(ui, &tr!("preset.steps_format"), serif(11.0, W::Italic), p.again);
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                caps_label(ui, &tr!("preset.relearning_steps"));
                if text_field_font(ui, &mut v.relearn, "10m", cw, 36.0, mono(12.5)).changed() {
                    v.dirty = true;
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                caps_label(ui, &tr!("preset.max_interval"));
                if number_field(ui, "maxivl", &mut v.draft.cfg.max_interval, &tr!("preset.days"), cw) {
                    v.dirty = true;
                }
            });
            ui.end_row();
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                caps_label(ui, &tr!("preset.new_per_day"));
                if number_field(ui, "newpd", &mut v.draft.cfg.new_per_day, "", cw) {
                    v.dirty = true;
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                caps_label(ui, &tr!("preset.reviews_per_day"));
                if number_field(ui, "revpd", &mut v.draft.cfg.reviews_per_day, "", cw) {
                    v.dirty = true;
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                caps_label(ui, &tr!("preset.leech"));
                let cur = tr!("preset.leech_value", n = v.draft.cfg.leech_threshold, action = tr!(if v.draft.cfg.leech_action == LeechAction::Tag { "preset.leech_tag" } else { "preset.leech_suspend" }));
                select_field(ui, "leech", &cur, None, cw, 36.0, |ui| {
                    for n in [4u32, 6, 8, 10, 12, 16] {
                        if menu_item(ui, &tr!("preset.lapses", n = n), v.draft.cfg.leech_threshold == n).clicked() {
                            v.draft.cfg.leech_threshold = n;
                            v.dirty = true;
                        }
                    }
                    menu_sep(ui);
                    if menu_item(ui, &tr!("preset.leech_tag_long"), v.draft.cfg.leech_action == LeechAction::Tag).clicked() {
                        v.draft.cfg.leech_action = LeechAction::Tag;
                        v.dirty = true;
                    }
                    if menu_item(ui, &tr!("preset.leech_suspend_long"), v.draft.cfg.leech_action == LeechAction::Suspend).clicked() {
                        v.draft.cfg.leech_action = LeechAction::Suspend;
                        v.dirty = true;
                    }
                });
            });
            ui.end_row();
        });
    });
}

fn order_card(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let v = app.preset_view.as_mut().unwrap();
    card_box(ui, |ui| {
        let w = ui.available_width();
        let cw = (w - 28.0) / 2.0;
        egui::Grid::new("order_grid").num_columns(2).spacing(vec2(28.0, 12.0)).min_col_width(cw).show(ui, |ui| {
            option_row(ui, &tr!("preset.review_order"), cw, |ui| {
                let cur = v.draft.cfg.review_order;
                inline_select(ui, "rev_order", &cur.label(), |ui| {
                    for o in ReviewOrder::ALL {
                        if menu_item(ui, &o.label(), o == cur).clicked() {
                            v.draft.cfg.review_order = o;
                            v.dirty = true;
                        }
                    }
                });
            });
            option_row(ui, &tr!("preset.bury"), cw, |ui| {
                if toggle(ui, &mut v.draft.cfg.bury_siblings, &tr!("preset.bury")).clicked() {
                    v.dirty = true;
                }
            });
            ui.end_row();
            option_row(ui, &tr!("preset.new_mix"), cw, |ui| {
                let cur = v.draft.cfg.new_mix;
                inline_select(ui, "new_mix", &cur.label(), |ui| {
                    for o in NewMix::ALL {
                        if menu_item(ui, &o.label(), o == cur).clicked() {
                            v.draft.cfg.new_mix = o;
                            v.dirty = true;
                        }
                    }
                });
            });
            option_row(ui, &tr!("preset.fuzz"), cw, |ui| {
                let mut on = v.draft.cfg.fuzz && v.draft.cfg.load_balance;
                if toggle(ui, &mut on, &tr!("preset.fuzz")).clicked() {
                    v.draft.cfg.fuzz = on;
                    v.draft.cfg.load_balance = on;
                    v.dirty = true;
                }
            });
            ui.end_row();
            option_row(ui, &tr!("preset.new_order"), cw, |ui| {
                let cur = v.draft.cfg.new_order;
                inline_select(ui, "new_order", &cur.label(), |ui| {
                    for o in NewOrder::ALL {
                        if menu_item(ui, &o.label(), o == cur).clicked() {
                            v.draft.cfg.new_order = o;
                            v.dirty = true;
                        }
                    }
                });
            });
            ui.end_row();
        });
    });
    let _ = p;
}

fn option_row(ui: &mut Ui, label_s: &str, w: f32, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    ui.allocate_ui_with_layout(vec2(w, 34.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(w);
        label(ui, label_s, serif(13.5, W::Regular), p.text2);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

fn optimizer_card(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let mut start: Option<&'static str> = None;
    let mut apply_params: Option<Vec<f32>> = None;
    {
        let v = app.preset_view.as_mut().unwrap();
        card_box(ui, |ui| {
            let since = v.draft.last_optimized_at.map(|t| (srs::now_ms() - t) / srs::DAY_MS);
            let suggested = v.reviews >= 400 && since.is_none_or(|d| d >= 30);
            ui.horizontal(|ui| {
                label(ui, &tr!("preset.optimizer"), serif(16.0, W::Semi), p.text);
                if suggested {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        label(ui, &tr!("preset.suggested"), serif(12.0, W::Regular), p.hard);
                    });
                }
            });
            let last = match since {
                None => tr!("preset.never_optimized"),
                Some(0) => tr!("preset.optimized_today"),
                Some(d) => tr!("preset.optimized_ago", n = d),
            };
            para(ui, &tr!("preset.reviews_here", n = thousands(v.reviews), last = last), serif(13.0, W::Regular), p.muted);
            // Atual × novo.
            let (t, _) = ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::hover());
            let c1 = t.right() - 78.0;
            let c2 = t.right();
            caps(ui.painter(), pos2(c1, t.top() + 8.0), Align2::RIGHT_CENTER, &tr!("preset.current_col"), p.dim);
            caps(ui.painter(), pos2(c2, t.top() + 8.0), Align2::RIGHT_CENTER, &tr!("preset.new_col"), p.dim);
            let new_eval = v.proposal.as_ref().and_then(|x| x.1);
            for (i, (lab, a, b, fmt)) in [
                ("Log loss", v.eval_now.map(|e| e.0), new_eval.map(|e| e.0), 0u8),
                ("RMSE (bins)", v.eval_now.map(|e| e.1), new_eval.map(|e| e.1), 1u8),
            ]
            .into_iter()
            .enumerate()
            {
                let y = t.top() + 36.0 + i as f32 * 30.0;
                if i == 0 {
                    ui.painter().hline(t.x_range(), y + 15.0, Stroke::new(1.0, p.line_row));
                }
                text(ui.painter(), pos2(t.left(), y), Align2::LEFT_CENTER, lab, serif(13.0, W::Regular), p.text2, 150.0);
                let f = |x: Option<f32>| x.map(|x| if fmt == 0 { srs::decimal(x as f64, 3) } else { pct(x, 1) }).unwrap_or_else(|| "—".into());
                text(ui.painter(), pos2(c1, y), Align2::RIGHT_CENTER, &f(a), serif(13.0, W::Regular), p.muted, 70.0);
                let better = matches!((a, b), (Some(a), Some(b)) if b <= a);
                text(ui.painter(), pos2(c2, y), Align2::RIGHT_CENTER, &f(b), serif(13.0, W::Semi), if better { p.good } else { p.text }, 70.0);
            }
            if let Some((params, _)) = &v.proposal {
                let same = params.iter().zip(fsrs::DEFAULT_PARAMETERS.iter()).all(|(a, b)| (a - b).abs() < 1e-4);
                if same {
                    para(ui, &tr!("preset.kept_defaults"), serif(12.0, W::Italic), p.hard);
                }
            }
            let shown: Vec<f32> = v.proposal.as_ref().map(|x| x.0.clone()).unwrap_or_else(|| if v.draft.params.is_empty() { fsrs::DEFAULT_PARAMETERS.to_vec() } else { v.draft.params.clone() });
            let w_txt = format!("w = [{}]", shown.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(", "));
            egui::Frame::NONE.fill(p.window).stroke(Stroke::new(1.0, p.border)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                para(ui, &w_txt, mono(10.5), p.muted);
            });
            if let (Some(kind), Some(prog)) = (v.running, &v.progress) {
                let (cur, total) = prog.lock().map(|s| (s.current(), s.total())).unwrap_or((0, 0));
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                let frac = if total > 0 { cur as f32 / total as f32 } else { 0.0 };
                progress(ui.painter(), Rect::from_min_size(r.left_center() - vec2(0.0, 2.0), vec2(r.width() - 90.0, 4.0)), frac, p.accent);
                text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &tr!(if kind == "optimize" { "preset.optimizing" } else { "preset.evaluating" }), serif(12.0, W::Italic), p.muted, 90.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let busy = v.running.is_some();
                if let Some((params, _)) = &v.proposal {
                    let w = ui.available_width() - 110.0;
                    if button_sized(ui, &tr!("preset.apply"), None, Btn::Primary, vec2(w, 34.0), !busy).clicked() {
                        apply_params = Some(params.clone());
                    }
                } else {
                    let w = ui.available_width() - 110.0;
                    if button_sized(ui, &tr!("preset.optimize"), None, Btn::Primary, vec2(w, 34.0), !busy && v.reviews > 0).clicked() {
                        start = Some("optimize");
                    }
                }
                if button_sized(ui, &tr!("preset.evaluate"), None, Btn::Secondary, vec2(100.0, 34.0), !busy && v.reviews > 0).clicked() {
                    start = Some("evaluate");
                }
            });
        });
    }
    if let Some(k) = start {
        start_job(app, k);
    }
    if let Some(params) = apply_params {
        let v = app.preset_view.as_mut().unwrap();
        v.draft.params = params;
        v.draft.last_optimized_at = Some(srs::now_ms());
        v.eval_now = v.proposal.take().and_then(|x| x.1);
        v.dirty = true;
        save_preset(app);
        recompute_memory(app);
        start_job(app, "simulate");
    }
}

/// Recalcula D e S de todos os cartões do preset com os parâmetros novos,
/// a partir do histórico (os vencimentos não mudam).
fn recompute_memory(app: &mut App) {
    let Some(v) = app.preset_view.as_ref() else { return };
    let preset = v.draft.clone();
    let decks: Vec<i64> = app.decks.iter().filter(|d| d.preset_id == preset.id).map(|d| d.id).collect();
    let Ok(f) = fsrs::FSRS::new(&preset.params) else { return };
    let histories = app.db.histories(&decks).unwrap_or_default();
    let mut n = 0;
    for (card_id, revs) in histories {
        let Some(start) = revs.iter().position(|r| r.state_before == srs::State::New) else { continue };
        let mut reviews = Vec::new();
        let mut last: Option<i64> = None;
        for r in &revs[start..] {
            let day = app.clock.day_of(r.at);
            reviews.push(FSRSReview { rating: r.rating.clamp(1, 4), delta_t: last.map(|d| (day - d).max(0) as u32).unwrap_or(0) });
            last = Some(day);
        }
        if let Ok(mem) = f.memory_state(FSRSItem { reviews }, None) {
            if let Ok(Some(mut c)) = app.db.card(card_id) {
                c.s.stability = Some(mem.stability);
                c.s.difficulty = Some(mem.difficulty);
                let _ = app.db.save_card(&c);
                n += 1;
            }
        }
    }
    app.changed();
    app.toast(tr!("toast.params_applied", n = n));
    let _ = db::new_id;
}

fn simulator_card(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let mut rerun = false;
    {
        let v = app.preset_view.as_mut().unwrap();
        card_frame().inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height() - 36.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                label(ui, &tr!("preset.simulator"), serif(16.0, W::Semi), p.text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, "↻", &tr!("preset.simulate_again"), 24.0).clicked() {
                        rerun = true;
                    }
                    label(ui, &tr!("preset.days_365"), serif(12.0, W::Regular), p.muted);
                });
            });
            let (chart, _) = ui.allocate_exact_size(vec2(ui.available_width(), 132.0), Sense::hover());
            let plot = Rect::from_min_max(chart.min, pos2(chart.right(), chart.bottom() - 14.0));
            ui.painter().hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, p.border));
            match &v.sim {
                Some(pts) if pts.len() > 2 => {
                    let max_load = pts.iter().map(|x| x.1).fold(0.0, f32::max).max(1.0);
                    let (min_mem, max_mem) = pts.iter().fold((f32::MAX, 0.0f32), |a, x| (a.0.min(x.2), a.1.max(x.2)));
                    let xn = |dr: f32| (dr - 0.70) / 0.29;
                    let load: Vec<(f32, f32)> = pts.iter().map(|x| (xn(x.0), x.1 / max_load * 0.92)).collect();
                    let mem: Vec<(f32, f32)> = pts.iter().map(|x| (xn(x.0), 0.25 + (x.2 - min_mem) / (max_mem - min_mem).max(1.0) * 0.6)).collect();
                    super::charts::curve(ui.painter(), plot, &load, p.hard, 2.0, false);
                    super::charts::curve(ui.painter(), plot, &mem, p.accent, 2.0, false);
                    let cx = plot.left() + xn(v.draft.cfg.desired_retention) * plot.width();
                    super::charts::dashed_v(ui.painter(), cx, plot.top() + 2.0, plot.bottom(), p.text2);
                }
                _ => {
                    text(ui.painter(), plot.center(), Align2::CENTER_CENTER, &tr!(if v.running == Some("simulate") { "preset.simulating" } else { "preset.no_simulation" }), serif(12.0, W::Italic), p.muted, plot.width());
                }
            }
            for t in [0.70f32, 0.90, 0.99] {
                let x = plot.left() + (t - 0.70) / 0.29 * plot.width();
                let al = if t < 0.71 { Align2::LEFT_CENTER } else if t > 0.98 { Align2::RIGHT_CENTER } else { Align2::CENTER_CENTER };
                text(ui.painter(), pos2(x, chart.bottom() - 4.0), al, &ret2(t), serif(10.0, W::Regular), p.muted, 40.0);
            }
            let (lr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
            super::charts::legend(ui.painter(), lr.left_center(), &[(&tr!("preset.daily_load"), p.hard), (&tr!("preset.retained"), p.accent)], true);
            if let Some(pts) = &v.sim {
                // Ponto a partir do qual cada ponto de retenção custa muito mais revisões do que acrescenta em memória.
                let knee = knee(pts);
                para(ui, &tr!("preset.knee", value = ret2(knee)), serif(12.5, W::Italic), p.text2);
            }
        });
    }
    if rerun {
        start_job(app, "simulate");
    }
}

/// Valor clicável "Texto ▾" sem moldura (as linhas de opções do design).
fn inline_select(ui: &mut Ui, id: &str, current: &str, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    let font = serif(13.5, W::Regular);
    let w = text_width(ui.painter(), current, font.clone()) + 22.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.expand2(vec2(6.0, 0.0)), CornerRadius::same(6), p.hover);
    }
    text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, current, font, p.text, w);
    text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, "▾", serif(11.0, W::Regular), p.dim, 14.0);
    a11y(&resp, egui::accesskit::Role::ComboBox, current, None);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&resp).id(egui::Id::unique(("inline_select", id))).frame(popup_frame()).width(220.0).show(add);
}

/// Retenção em que o custo marginal (revisões por cartão lembrado a mais) passa
/// de 3× o custo médio da faixa — o "joelho" da curva.
fn knee(pts: &[(f32, f32, f32)]) -> f32 {
    // Suaviza em janelas de 4 passos (a simulação tem ruído) e só procura a partir de 0,80.
    if pts.len() < 8 {
        return 0.9;
    }
    let first = pts[0];
    let last = pts[pts.len() - 1];
    let avg = ((last.1 - first.1) / (last.2 - first.2).max(1e-3)).max(1e-3);
    for i in 0..pts.len().saturating_sub(4) {
        let (a, b) = (pts[i], pts[i + 4]);
        if a.0 < 0.795 {
            continue;
        }
        let dm = b.2 - a.2;
        let dl = b.1 - a.1;
        if dm <= 0.0 || dl / dm > 3.0 * avg {
            return a.0;
        }
    }
    0.95
}
