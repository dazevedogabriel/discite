//! Janela do Discite, implementada a partir do canvas "Discite — Interface":
//! barra de título própria, barra lateral (inteira ou recolhida), Painel,
//! Estatísticas, Cartões, Disciplina (Caderno · Baralhos · Estatísticas),
//! Sessão de revisão, Opções do preset, Configurações, editor de nota,
//! paleta de comandos e avisos.

mod browser;
mod charts;
mod dashboard;
mod modals;
mod note_editor;
mod page;
mod palette;
mod preset;
mod review;
mod richtext;
mod settings;
mod stats_view;
mod subject;
mod theme;
mod widgets;

use crate::db::{self, Counts, Db, Deck, Preset, Subject};
use crate::srs::{self, Clock};
use crate::{tr, tr_n};
use anyhow::Result;
use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Key, Modifiers, Rect, ResizeDirection, Sense, Stroke, ViewportCommand, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use theme::*;
use widgets::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() -> Result<()> {
    // Inglês até as configurações serem lidas (o banco guarda o idioma escolhido).
    crate::i18n::set_language(Some("en"));
    let root = db::data_root();
    let db = Db::open(&root)?;
    if std::env::var_os("DISCITE_DEMO").is_some() && db.list_subjects()?.is_empty() {
        settings::seed_demo(&db)?;
    }
    let viewport = egui::ViewportBuilder::default()
        .with_title("Discite")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([1024.0, 640.0])
        .with_icon(window_icon());
    // Windows/Linux: sem moldura, com a barra de título do design. macOS: barra
    // nativa transparente sobre o conteúdo, para manter os três botões do sistema.
    let viewport = if MAC {
        viewport.with_decorations(true).with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        viewport.with_decorations(false)
    };
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native("Discite", options, Box::new(move |cc| Ok(Box::new(App::new(cc, db)))))
        .map_err(|e| anyhow::anyhow!(tr!("error.window", error = e)))
}

pub const LOGO_SVG: &[u8] = include_bytes!("../../assets/logo.svg");

/// `assets/logo.svg`, rasterizado pelo carregador SVG do egui_extras.
fn logo(ui: &egui::Ui, rect: Rect) {
    egui::Image::from_bytes("bytes://logo.svg", LOGO_SVG).paint_at(ui, rect);
}

/// Ícone gerado pelo `build.rs` a partir do mesmo SVG (igual ao do executável).
fn window_icon() -> egui::IconData {
    let png = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.png"));
    match image::load_from_memory(png) {
        Ok(img) => {
            let img = img.to_rgba8();
            egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
        }
        Err(_) => egui::IconData::default(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub rollover_hour: u32,
    pub sidebar_collapsed: bool,
    pub auto_backup: bool,
    pub last_backup: i64,
    /// Largura da coluna de leitura do caderno.
    pub reading_width: f32,
    /// Teto do cronômetro por cartão (s).
    pub answer_cap_secs: u32,
    pub last_deck: Option<i64>,
    /// Código do idioma (`en`, `pt-BR`…); `None` segue o sistema.
    pub lang: Option<String>,
    /// No caderno a barra lateral fica recolhida (como no design) até o usuário abrir.
    pub notebook_sidebar_open: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { dark: true, rollover_hour: 4, sidebar_collapsed: false, auto_backup: true, last_backup: 0, reading_width: 640.0, answer_cap_secs: 60, last_deck: None, notebook_sidebar_open: false, lang: Some("en".into()) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Notebook,
    Decks,
    Stats,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Dashboard,
    Stats,
    Browser,
    Subject(i64, Tab),
    Review,
    Preset(i64),
    Settings,
}

pub struct App {
    pub db: Db,
    pub cfg: Settings,
    pub clock: Clock,
    pub screen: Screen,
    /// Tela para onde "Sair" da revisão volta.
    pub back_screen: Screen,
    pub subjects: Vec<Subject>,
    pub decks: Vec<Deck>,
    pub presets: Vec<Preset>,
    raw_counts: HashMap<i64, Counts>,
    done_today: HashMap<i64, (u32, u32)>,
    limited: HashMap<i64, Counts>,
    counts_at: Instant,
    counts_day: i64,
    pub avg_secs: f32,
    pub streak: u32,
    /// Sobe a cada mudança no banco; os caches das telas comparam com ele.
    pub data_version: u64,
    toasts: Vec<(String, bool, Instant)>,
    pub modal: Option<modals::Modal>,
    pub note_editor: Option<note_editor::NoteEditor>,
    pub page: Option<page::PageEditor>,
    pub session: Option<review::Session>,
    pub browser: browser::BrowserState,
    pub stats: stats_view::StatsState,
    pub dash: dashboard::DashState,
    pub preset_view: Option<preset::PresetView>,
    pub palette: Option<palette::Palette>,
    pub subject_view: subject::SubjectState,
    /// Imagem colada neste quadro (Ctrl+V com imagem).
    pub pasted_image: Option<String>,
    /// Arquivos soltos na janela neste quadro.
    pub dropped: Vec<std::path::PathBuf>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, db: Db) -> App {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        install_fonts(&cc.egui_ctx);
        let cfg: Settings = db.meta("settings").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        crate::i18n::set_language(cfg.lang.as_deref());
        apply(&cc.egui_ctx, cfg.dark);
        let clock = Clock { rollover_hour: cfg.rollover_hour };
        let mut app = App {
            db,
            clock,
            screen: Screen::Dashboard,
            back_screen: Screen::Dashboard,
            subjects: vec![],
            decks: vec![],
            presets: vec![],
            raw_counts: HashMap::new(),
            done_today: HashMap::new(),
            limited: HashMap::new(),
            counts_at: Instant::now(),
            counts_day: 0,
            avg_secs: 10.0,
            streak: 0,
            data_version: 1,
            toasts: vec![],
            modal: None,
            note_editor: None,
            page: None,
            session: None,
            browser: Default::default(),
            stats: Default::default(),
            dash: Default::default(),
            preset_view: None,
            palette: None,
            subject_view: Default::default(),
            pasted_image: None,
            dropped: vec![],
            cfg,
        };
        let _ = app.db.unbury();
        app.counts_day = app.clock.today();
        app.reload();
        app.maybe_backup(false);
        app
    }

    pub fn save_settings(&mut self) {
        self.clock = Clock { rollover_hour: self.cfg.rollover_hour };
        if let Ok(s) = serde_json::to_string(&self.cfg) {
            let _ = self.db.set_meta("settings", &s);
        }
    }

    /// Recarrega disciplinas, baralhos, presets e contagens.
    pub fn reload(&mut self) {
        match (self.db.list_subjects(), self.db.list_decks(), self.db.list_presets()) {
            (Ok(s), Ok(d), Ok(p)) => {
                self.subjects = s;
                self.decks = d;
                self.presets = p;
            }
            (a, b, c) => {
                if let Some(e) = a.err().or(b.err()).or(c.err()) {
                    self.error(e);
                }
            }
        }
        self.data_version += 1;
        self.refresh_counts();
    }

    pub fn refresh_counts(&mut self) {
        let now = srs::now_ms();
        let today = self.clock.today();
        if today != self.counts_day {
            // Virou o dia de estudo: desenterra os cartões.
            self.counts_day = today;
            let _ = self.db.unbury();
        }
        let day_start = self.clock.day_start(today);
        self.raw_counts = self.db.deck_counts(now + srs::LEARN_AHEAD_MS, self.clock.today_end()).unwrap_or_default();
        self.done_today = self.db.done_today(day_start).unwrap_or_default();
        self.limited.clear();
        let roots: Vec<i64> = self.decks.iter().filter(|d| d.parent_id.is_none()).map(|d| d.id).collect();
        for r in roots {
            self.compute_limited(r);
        }
        self.counts_at = Instant::now();
        self.avg_secs = crate::stats::avg_answer_secs(&self.db);
        let days = crate::stats::per_day(&self.db, &crate::stats::Scope(None), &self.clock, self.clock.day_start(today - 400)).unwrap_or_default();
        self.streak = crate::stats::streak(&days, today);
    }

    fn compute_limited(&mut self, deck: i64) -> Counts {
        let mut c = self.raw_counts.get(&deck).copied().unwrap_or_default();
        let kids: Vec<i64> = self.decks.iter().filter(|d| d.parent_id == Some(deck)).map(|d| d.id).collect();
        for k in kids {
            let kc = self.compute_limited(k);
            c.new += kc.new;
            c.learn += kc.learn;
            c.review += kc.review;
        }
        let cfg = &self.preset_of_deck(deck).cfg;
        let (mut intro, mut revs) = (0, 0);
        for id in db::subtree(&self.decks, deck) {
            if let Some((a, b)) = self.done_today.get(&id) {
                intro += a;
                revs += b;
            }
        }
        c.new = c.new.min(cfg.new_per_day.saturating_sub(intro));
        c.review = c.review.min(cfg.reviews_per_day.saturating_sub(revs));
        self.limited.insert(deck, c);
        c
    }

    pub fn deck_counts(&self, deck: i64) -> Counts {
        self.limited.get(&deck).copied().unwrap_or_default()
    }

    pub fn subject_counts(&self, subject: i64) -> Counts {
        let mut c = Counts::default();
        for d in self.decks.iter().filter(|d| d.subject_id == subject && d.parent_id.is_none()) {
            let k = self.deck_counts(d.id);
            c.new += k.new;
            c.learn += k.learn;
            c.review += k.review;
        }
        c
    }

    pub fn total_counts(&self) -> Counts {
        let mut c = Counts::default();
        for s in &self.subjects {
            let k = self.subject_counts(s.id);
            c.new += k.new;
            c.learn += k.learn;
            c.review += k.review;
        }
        c
    }

    pub fn preset_of_deck(&self, deck: i64) -> &Preset {
        let pid = self.decks.iter().find(|d| d.id == deck).map(|d| d.preset_id).unwrap_or(1);
        self.presets.iter().find(|p| p.id == pid).or(self.presets.first()).expect("há sempre um preset")
    }

    pub fn preset(&self, id: i64) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id == id)
    }

    pub fn subject(&self, id: i64) -> Option<&Subject> {
        self.subjects.iter().find(|s| s.id == id)
    }

    pub fn deck(&self, id: i64) -> Option<&Deck> {
        self.decks.iter().find(|d| d.id == id)
    }

    pub fn subject_color_of_deck(&self, deck: i64) -> Color32 {
        self.deck(deck).and_then(|d| self.subject(d.subject_id)).map(|s| subject_color(s.color)).unwrap_or(pal().accent)
    }

    pub fn root_decks(&self, subject: i64) -> Vec<Deck> {
        self.decks.iter().filter(|d| d.subject_id == subject && d.parent_id.is_none()).cloned().collect()
    }

    /// "Farmacologia › Cardiovascular › Anti-hipertensivos"
    pub fn deck_path(&self, id: i64) -> String {
        crate::search::deck_path(&self.decks, id).replace("::", " › ")
    }

    pub fn subject_decks(&self, subject: i64) -> Vec<i64> {
        self.decks.iter().filter(|d| d.subject_id == subject).map(|d| d.id).collect()
    }

    pub fn decay_of_deck(&self, deck: i64) -> f32 {
        let p = self.preset_of_deck(deck);
        if p.params.len() >= 21 { p.params[20] } else { fsrs::FSRS6_DEFAULT_DECAY }
    }

    /// Estimativa de minutos para `n` cartões.
    pub fn minutes_for(&self, n: u32) -> u32 {
        ((n as f32 * self.avg_secs) / 60.0).round() as u32
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), false, Instant::now()));
    }

    pub fn error(&mut self, e: impl std::fmt::Display) {
        self.toasts.push((format!("{e:#}"), true, Instant::now()));
    }

    /// Registra uma mudança no banco: invalida caches e recalcula contagens.
    pub fn changed(&mut self) {
        self.data_version += 1;
        self.refresh_counts();
    }

    pub fn go(&mut self, screen: Screen) {
        if let Some(p) = self.page.as_mut() {
            p.flush(&self.db);
        }
        if let Screen::Subject(sid, Tab::Notebook) = screen {
            self.subject_view.last_tab.insert(sid, Tab::Notebook);
        } else if let Screen::Subject(sid, t) = screen {
            self.subject_view.last_tab.insert(sid, t);
        }
        self.screen = screen;
    }

    pub fn open_subject(&mut self, sid: i64) {
        let tab = self.subject_view.last_tab.get(&sid).copied().unwrap_or(Tab::Decks);
        self.go(Screen::Subject(sid, tab));
    }

    pub fn open_page(&mut self, page_id: i64) {
        let Ok(Some(meta)) = self.db.page(page_id) else {
            self.toast(tr!("toast.page_not_found"));
            return;
        };
        if let Some(p) = self.page.as_mut() {
            p.flush(&self.db);
        }
        match page::PageEditor::open(&self.db, meta.clone()) {
            Ok(ed) => {
                self.page = Some(ed);
                self.subject_view.last_page.insert(meta.subject_id, page_id);
                self.go(Screen::Subject(meta.subject_id, Tab::Notebook));
            }
            Err(e) => self.error(e),
        }
    }

    pub fn new_page(&mut self, subject: i64, parent: Option<i64>) {
        match self.db.add_page(subject, parent, &tr!("page.untitled")) {
            Ok(id) => {
                self.data_version += 1;
                self.open_page(id);
                if let Some(p) = self.page.as_mut() {
                    p.focus_title = true;
                }
            }
            Err(e) => self.error(e),
        }
    }

    /// Assunto "atual" para atalhos (Ctrl+N, Ctrl+Shift+A).
    pub fn current_subject(&self) -> Option<i64> {
        match self.screen {
            Screen::Subject(s, _) => Some(s),
            _ => self.subjects.first().map(|s| s.id),
        }
    }

    pub fn default_deck(&self) -> Option<i64> {
        if let Screen::Subject(s, _) = self.screen {
            if let Some(d) = self.cfg.last_deck.filter(|d| self.deck(*d).is_some_and(|d| d.subject_id == s)) {
                return Some(d);
            }
            return self.root_decks(s).first().map(|d| d.id);
        }
        self.cfg.last_deck.filter(|d| self.deck(*d).is_some()).or_else(|| self.decks.first().map(|d| d.id))
    }

    pub fn add_note(&mut self) {
        match self.default_deck() {
            Some(d) => self.note_editor = Some(note_editor::NoteEditor::new_note(d, None)),
            None => self.toast(tr!("toast.create_subject_first_notes")),
        }
    }

    /// Cópia de segurança diária (e ao fechar), mantendo as 10 mais recentes.
    pub fn maybe_backup(&mut self, force: bool) {
        if !self.cfg.auto_backup && !force {
            return;
        }
        let now = srs::now_ms();
        if !force && now - self.cfg.last_backup < 6 * 3_600_000 {
            return;
        }
        let dir = self.db.root.join("backups");
        let _ = std::fs::create_dir_all(&dir);
        let name = chrono::Local::now().format("discite-%Y%m%d-%H%M%S.db").to_string();
        match self.db.backup_to(&dir.join(name)) {
            Ok(()) => {
                self.cfg.last_backup = now;
                self.save_settings();
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    let mut files: Vec<_> = rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("discite-")).map(|e| e.path()).collect();
                    files.sort();
                    while files.len() > 10 {
                        let _ = std::fs::remove_file(files.remove(0));
                    }
                }
            }
            Err(e) => self.error(tr!("toast.backup_failed", error = format!("{e:#}"))),
        }
    }

    fn collect_input(&mut self, ctx: &egui::Context) {
        self.pasted_image = None;
        self.dropped.clear();
        let mut pasted = None;
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::PasteImage(img) = e {
                    pasted = Some(img.clone());
                }
            }
            for f in &i.raw.dropped_files {
                let p = f.path();
                if !p.as_os_str().is_empty() {
                    self.dropped.push(p.to_path_buf());
                }
            }
        });
        if let Some(img) = pasted {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            match crate::media::store_rgba(&self.db.media_dir(), img.size[0] as u32, img.size[1] as u32, rgba) {
                Ok(s) => {
                    let _ = self.db.add_media(&s);
                    self.pasted_image = Some(s.file_name);
                }
                Err(e) => self.error(e),
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input();
        let overlay = self.modal.is_some() || self.note_editor.is_some() || self.palette.is_some();
        let cmd = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, k));
        let cmd_shift = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, k));
        if cmd(Key::K) {
            self.palette = if self.palette.is_some() { None } else { Some(palette::Palette::default()) };
        }
        if overlay {
            return;
        }
        if cmd_shift(Key::A) {
            self.add_note();
        }
        if cmd(Key::Backslash) {
            if matches!(self.screen, Screen::Subject(_, Tab::Notebook)) {
                self.cfg.notebook_sidebar_open = !self.cfg.notebook_sidebar_open;
            } else {
                self.cfg.sidebar_collapsed = !self.cfg.sidebar_collapsed;
            }
            self.save_settings();
        }
        if self.screen != Screen::Review {
            if cmd(Key::N) {
                match self.current_subject() {
                    Some(s) => self.new_page(s, None),
                    None => self.toast(tr!("toast.create_subject_first")),
                }
            }
            if !matches!(self.screen, Screen::Subject(_, Tab::Notebook)) && cmd(Key::F) {
                self.go(Screen::Browser);
                self.browser.focus_search = true;
            }
        }
        if self.screen == Screen::Dashboard && !typing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)) {
            review::start(self, review::Scope::All);
        }
    }
}

// ───────────────────────────── barra de título ─────────────────────────────

fn title_bar(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let p = pal();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0, p.titlebar);
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line));
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let btn_w = 40.0;
    // No Mac os botões são os do sistema (à esquerda); aqui não desenhamos os nossos.
    let buttons_left = if MAC { rect.right() - 14.0 } else { rect.right() - 5.0 - 3.0 * btn_w - 4.0 };
    let theme_w = 96.0;
    let drag = ui.interact(Rect::from_min_max(rect.min, pos2(buttons_left - theme_w - 70.0, rect.bottom())), ui.scope_id().with("drag"), Sense::click_and_drag());
    if drag.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if drag.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }

    // No Mac a barra fica fina como a nativa, sem logo nem nome (o logo vai para a barra lateral).
    if !MAC {
        let mark = Rect::from_center_size(pos2(rect.left() + 21.5, rect.center().y), vec2(20.0, 20.0));
        logo(ui, mark);
        caps_sized(&painter, pos2(mark.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, &format!("Discite {VERSION}"), 11.0, p.dim);
    }
    let btn_h = rect.height() - 10.0;
    let btn_top = rect.top() + 5.0;

    // Tema claro/escuro.
    let theme_r = Rect::from_min_size(pos2(buttons_left - theme_w - 60.0, btn_top), vec2(theme_w, btn_h));
    let tresp = ui.interact(theme_r, ui.scope_id().with("theme"), Sense::click());
    if tresp.hovered() {
        painter.rect_filled(theme_r, CornerRadius::same(6), p.hover);
    }
    let label = tr!(if app.cfg.dark { "app.theme_dark" } else { "app.theme_light" });
    text(&painter, theme_r.center(), Align2::CENTER_CENTER, &label, serif(11.5, W::Regular), if tresp.hovered() { p.text } else { p.muted }, theme_w);
    a11y(&tresp, egui::accesskit::Role::Button, &tr!("app.toggle_theme"), None);
    if tresp.clicked() {
        app.cfg.dark = !app.cfg.dark;
        app.save_settings();
        apply(&ctx, app.cfg.dark);
    }
    // Idioma: o código atual; clicar abre a lista de idiomas.
    let code = crate::i18n::current().to_uppercase();
    let lw = text_width(&painter, &code, serif(11.0, W::Semi)) + 20.0;
    let lr = Rect::from_min_size(pos2(buttons_left - 8.0 - lw, btn_top), vec2(lw, btn_h));
    let lresp = ui.interact(lr, ui.scope_id().with("lang"), Sense::click());
    if lresp.hovered() {
        painter.rect_filled(lr, CornerRadius::same(6), p.hover);
    }
    caps_sized(&painter, lr.center(), Align2::CENTER_CENTER, &code, 11.0, if lresp.hovered() { p.text } else { p.dim });
    a11y(&lresp, egui::accesskit::Role::Button, &tr!("app.language"), None);
    egui::Popup::menu(&lresp).frame(popup_frame()).width(200.0).show(|ui| settings::language_menu(app, ui));

    let window_buttons: &[&str] = if MAC { &[] } else { &["min", "max", "close"] };
    for (i, &kind) in window_buttons.iter().enumerate() {
        let r = Rect::from_min_size(pos2(buttons_left + i as f32 * btn_w, rect.top() + 5.0), vec2(btn_w, 28.0));
        let resp = ui.interact(r, ui.scope_id().with(kind), Sense::click());
        a11y(&resp, egui::accesskit::Role::Button, &tr!(match kind { "min" => "app.minimize", "max" => "app.maximize", _ => "app.close" }), None);
        let (bg, fg) = match (kind, resp.hovered()) {
            ("close", true) => (Color32::from_rgb(0xE5, 0x48, 0x4D), Color32::WHITE),
            (_, true) => (p.hover, p.text),
            _ => (Color32::TRANSPARENT, p.muted),
        };
        painter.rect_filled(r, CornerRadius::same(6), bg);
        let c = r.center();
        let s = Stroke::new(1.0, fg);
        match kind {
            "min" => {
                painter.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], s);
            }
            "max" => {
                painter.rect_stroke(Rect::from_center_size(c, vec2(8.0, 8.0)), 0, s, egui::StrokeKind::Inside);
            }
            _ => {
                painter.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
                painter.line_segment([c + vec2(4.0, -4.0), c + vec2(-4.0, 4.0)], s);
            }
        }
        if resp.clicked() {
            ctx.send_viewport_cmd(match kind {
                "min" => ViewportCommand::Minimized(true),
                "max" => ViewportCommand::Maximized(!maximized),
                _ => ViewportCommand::Close,
            });
        }
    }
}

fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.content_rect();
    let m = 5.0;
    let (l, rt, t, b) = (pos.x - r.left() < m, r.right() - pos.x < m, pos.y - r.top() < m, r.bottom() - pos.y < m);
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (.., true) => ResizeDirection::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        _ => CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

// ───────────────────────────── barra lateral ─────────────────────────────

fn nav_row(ui: &mut egui::Ui, num: &str, label: &str, on: bool) -> egui::Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, CornerRadius::same(8), p.selected);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(8), p.hover);
    }
    text(painter, pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, num, mono(10.5), if on { p.accent } else { p.dim }, 20.0);
    text(painter, pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, serif(14.0, if on { W::Semi } else { W::Regular }), if on { p.text } else { p.text2 }, rect.width() - 40.0);
    a11y(&resp, egui::accesskit::Role::Link, label, Some(on));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn sidebar(app: &mut App, ui: &mut egui::Ui) {
    let p = pal();
    let full = ui.max_rect();
    ui.painter().vline(full.right() - 0.5, full.y_range(), Stroke::new(1.0, p.line));
    let inner = Rect::from_min_max(full.min + vec2(16.0, 22.0), full.max - vec2(16.0, 16.0));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut ui;
    ui.spacing_mut().item_spacing.y = 2.0;

    // Marca.
    {
        let (r, _) = ui.allocate_exact_size(vec2(inner.width(), 52.0), Sense::hover());
        let g1 = line(ui.painter(), "Dis", serif(25.0, W::Bold), p.text, 200.0, -0.5);
        // No Mac a barra de título é só a faixa dos semáforos, então o logo vem para cá.
        let x0 = if MAC {
            logo(ui, Rect::from_min_size(pos2(r.left() + 4.0, r.top() + 3.0), vec2(40.0, 40.0)));
            r.left() + 54.0
        } else {
            r.left() + 6.0
        };
        ui.painter().galley(pos2(x0, r.top()), g1.clone(), p.text);
        let g2 = line(ui.painter(), "cite", serif(25.0, W::SemiItalic), p.accent, 200.0, -0.5);
        ui.painter().galley(pos2(x0 + g1.size().x, r.top()), g2, p.accent);
        caps_sized(ui.painter(), pos2(x0, r.top() + g1.size().y + 8.0), Align2::LEFT_CENTER, &tr!("app.tagline", version = &VERSION[..3]), 9.5, p.dim);
    }
    ui.add_space(20.0);

    let scr = app.screen;
    if nav_row(ui, "01", &tr!("nav.dashboard"), scr == Screen::Dashboard).clicked() {
        app.go(Screen::Dashboard);
    }
    if nav_row(ui, "02", &tr!("nav.stats"), scr == Screen::Stats).clicked() {
        app.go(Screen::Stats);
    }
    if nav_row(ui, "03", &tr!("nav.cards"), scr == Screen::Browser).clicked() {
        app.go(Screen::Browser);
    }
    ui.add_space(22.0);

    {
        let (r, _) = ui.allocate_exact_size(vec2(inner.width(), 22.0), Sense::hover());
        caps(ui.painter(), pos2(r.left() + 10.0, r.top() + 6.0), Align2::LEFT_CENTER, &tr!("nav.subjects"), p.dim);
        caps(ui.painter(), pos2(r.right() - 10.0, r.top() + 6.0), Align2::RIGHT_CENTER, &tr!("nav.today"), p.dim);
    }

    let footer_h = 74.0 + if app.session.is_some() && app.screen != Screen::Review { 44.0 } else { 0.0 };
    let list_h = (ui.available_height() - footer_h).max(60.0);
    egui::ScrollArea::vertical().id_salt("sb_subjects").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let subjects = app.subjects.clone();
        for s in &subjects {
            let active = matches!(scr, Screen::Subject(id, _) if id == s.id);
            let c = app.subject_counts(s.id);
            let n = c.new + c.learn + c.review;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(rect, CornerRadius::same(8), p.hover);
            }
            painter.circle_filled(pos2(rect.left() + 14.0, rect.center().y), 4.0, subject_color(s.color));
            text(painter, pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, &s.name, serif(14.0, if active { W::Semi } else { W::Regular }), if active { p.text } else { p.text2 }, rect.width() - 64.0);
            text(painter, pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, &n.to_string(), serif(12.0, W::Regular), if n > 0 { p.muted } else { p.dim }, 40.0);
            a11y(&resp, egui::accesskit::Role::Link, &tr!("nav.subject_due", name = s.name, n = n), Some(active));
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            if resp.clicked() {
                app.open_subject(s.id);
            }
            resp.context_menu(|ui| subject_menu(app, ui, s));
            if active {
                for (key, tab) in [("nav.notebook", Tab::Notebook), ("nav.decks", Tab::Decks), ("nav.stats", Tab::Stats)] {
                    let label = &tr!(key);
                    let on = matches!(scr, Screen::Subject(_, t) if t == tab);
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if on {
                        ui.painter().rect_filled(r, CornerRadius::same(7), p.accent_soft);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(7), p.hover);
                    }
                    text(ui.painter(), pos2(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, label, serif(13.0, if on { W::Semi } else { W::Regular }), if on { p.text } else { p.muted }, r.width() - 34.0);
                    a11y(&resp, egui::accesskit::Role::Link, label, Some(on));
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        if tab == Tab::Notebook {
                            subject::open_notebook(app, s.id);
                        } else {
                            app.go(Screen::Subject(s.id, tab));
                        }
                    }
                }
            }
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
        }
        text(ui.painter(), pos2(rect.left() + 14.0, rect.center().y), Align2::CENTER_CENTER, "+", serif(15.0, W::Regular), p.muted, 20.0);
        text(ui.painter(), pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, &tr!("nav.subject"), serif(14.0, W::Regular), p.muted, 120.0);
        a11y(&resp, egui::accesskit::Role::Button, &tr!("nav.new_subject"), None);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            app.modal = Some(modals::Modal::Subject { id: None, name: String::new(), color: app.subjects.len() % 8 });
        }
    });

    // Rodapé.
    let bottom = Rect::from_min_max(pos2(inner.left(), inner.bottom() - footer_h), inner.max);
    let mut f = ui.new_child(egui::UiBuilder::new().max_rect(bottom).layout(egui::Layout::top_down(egui::Align::Min)));
    if app.session.is_some() && app.screen != Screen::Review {
        let left = app.session.as_ref().map(|s| s.remaining()).unwrap_or(0);
        if button_sized(&mut f, &tr!("nav.resume_review", n = left), None, Btn::Primary, vec2(inner.width(), 34.0), true).clicked() {
            app.go(Screen::Review);
        }
        f.add_space(10.0);
    }
    let r = f.allocate_exact_size(vec2(inner.width(), 1.0), Sense::hover()).0;
    f.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, p.line));
    f.add_space(12.0);
    let (r, _) = f.allocate_exact_size(vec2(inner.width(), 20.0), Sense::hover());
    f.painter().circle_filled(pos2(r.left() + 13.5, r.center().y), 3.5, if app.streak > 0 { p.good } else { p.dim });
    let streak = match app.streak {
        0 => tr!("nav.streak_none"),
        n => tr_n!("nav.streak", n),
    };
    text(f.painter(), pos2(r.left() + 25.0, r.center().y), Align2::LEFT_CENTER, &streak, serif(13.0, W::Regular), p.text2, r.width() - 25.0);
    f.add_space(4.0);
    let (r, _) = f.allocate_exact_size(vec2(inner.width(), 18.0), Sense::hover());
    let backup = if app.cfg.last_backup > 0 { tr!("nav.backup_ago", when = ago(app.cfg.last_backup)) } else { tr!("nav.no_backup") };
    text(f.painter(), pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, &backup, serif(12.0, W::Italic), p.muted, r.width());
}

fn subject_menu(app: &mut App, ui: &mut egui::Ui, s: &Subject) {
    if menu_item(ui, &tr!("nav.open_notebook"), false).clicked() {
        subject::open_notebook(app, s.id);
    }
    if menu_item(ui, &tr!("nav.study"), false).clicked() {
        review::start(app, review::Scope::Subject(s.id));
    }
    menu_sep(ui);
    if menu_item(ui, &tr!("nav.rename_subject"), false).clicked() {
        app.modal = Some(modals::Modal::Subject { id: Some(s.id), name: s.name.clone(), color: s.color });
    }
    let idx = app.subjects.iter().position(|x| x.id == s.id).unwrap_or(0);
    if idx > 0 && menu_item(ui, &tr!("common.move_up"), false).clicked() {
        swap_subjects(app, idx, idx - 1);
    }
    if idx + 1 < app.subjects.len() && menu_item(ui, &tr!("common.move_down"), false).clicked() {
        swap_subjects(app, idx, idx + 1);
    }
    menu_sep(ui);
    if menu_item(ui, &tr!("nav.delete_subject"), false).clicked() {
        app.modal = Some(modals::Modal::DeleteSubject(s.id));
    }
}

fn swap_subjects(app: &mut App, a: usize, b: usize) {
    let mut sa = app.subjects[a].clone();
    let mut sb = app.subjects[b].clone();
    std::mem::swap(&mut sa.position, &mut sb.position);
    if sa.position == sb.position {
        sa.position = b as i64;
        sb.position = a as i64;
    }
    let _ = app.db.update_subject(&sa);
    let _ = app.db.update_subject(&sb);
    app.reload();
}

/// Barra recolhida (60 px) do design da página.
fn sidebar_collapsed(app: &mut App, ui: &mut egui::Ui) {
    let p = pal();
    let full = ui.max_rect();
    ui.painter().vline(full.right() - 0.5, full.y_range(), Stroke::new(1.0, p.line));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(full.shrink2(vec2(10.0, 16.0))).layout(egui::Layout::top_down(egui::Align::Center)));
    let ui = &mut ui;
    ui.spacing_mut().item_spacing.y = 6.0;
    let (r, _) = ui.allocate_exact_size(vec2(40.0, 36.0), Sense::hover());
    logo(ui, Rect::from_center_size(r.center() - vec2(0.0, 4.0), vec2(30.0, 30.0)));
    let scr = app.screen;
    for (num, key, target) in [("01", "nav.dashboard", Screen::Dashboard), ("02", "nav.stats", Screen::Stats), ("03", "nav.cards", Screen::Browser)] {
        let label = &tr!(key);
        let on = scr == target;
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 36.0), Sense::click());
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(8), p.selected);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(8), p.hover);
        }
        text(ui.painter(), r.center(), Align2::CENTER_CENTER, num, mono(10.5), if on { p.accent } else { p.dim }, 40.0);
        a11y(&resp, egui::accesskit::Role::Link, label, Some(on));
        if resp.on_hover_text(label).clicked() {
            app.go(target);
        }
    }
    let (r, _) = ui.allocate_exact_size(vec2(28.0, 21.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.line));
    for s in app.subjects.clone() {
        let on = matches!(scr, Screen::Subject(id, _) if id == s.id);
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 36.0), Sense::click());
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(8), p.selected);
            ui.painter().circle_filled(r.center(), 8.0, p.accent_soft);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(8), p.hover);
        }
        ui.painter().circle_filled(r.center(), 5.0, subject_color(s.color));
        let c = app.subject_counts(s.id);
        let label = tr!("nav.subject_due", name = s.name, n = c.new + c.learn + c.review);
        a11y(&resp, egui::accesskit::Role::Link, &label, Some(on));
        if resp.on_hover_text(label).clicked() {
            app.open_subject(s.id);
        }
    }
    let rest = ui.available_height() - 36.0;
    if rest > 0.0 {
        ui.add_space(rest);
    }
    if icon_button(ui, "»", &tr!("nav.expand_sidebar"), 36.0).clicked() {
        if matches!(app.screen, Screen::Subject(_, Tab::Notebook)) {
            app.cfg.notebook_sidebar_open = true;
        } else {
            app.cfg.sidebar_collapsed = false;
        }
        app.save_settings();
    }
}

// ───────────────────────────── cabeçalhos ─────────────────────────────

/// Cabeçalho de 76 px: título, subtítulo e controles à direita.
pub fn header(ui: &mut egui::Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut egui::Ui)) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line));
    let left = Rect::from_min_max(rect.min + vec2(28.0, 0.0), pos2(rect.right() - 28.0, rect.bottom()));
    text(ui.painter(), pos2(left.left(), rect.center().y - 10.0), Align2::LEFT_CENTER, title, serif(22.0, W::Semi), p.text, left.width() * 0.6);
    text(ui.painter(), pos2(left.left(), rect.center().y + 14.0), Align2::LEFT_CENTER, subtitle, serif(13.0, W::Regular), p.muted, left.width() * 0.6);
    let mut r = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::right_to_left(egui::Align::Center)));
    r.spacing_mut().item_spacing.x = 10.0;
    right(&mut r);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let p = pal();
        self.collect_input(&ctx);
        self.shortcuts(&ctx);
        if self.counts_at.elapsed() > Duration::from_secs(20) {
            self.refresh_counts();
        }
        preset::poll_jobs(self);

        // 28 pt no Mac: a altura da barra nativa, com os semáforos centralizados.
        egui::Panel::top("titlebar").exact_size(if MAC { 28.0 } else { 38.0 }).frame(egui::Frame::NONE).show_separator_line(false).show(ui, |ui| title_bar(self, ui));

        let review = self.screen == Screen::Review;
        if !review {
            let collapsed = if matches!(self.screen, Screen::Subject(_, Tab::Notebook)) { !self.cfg.notebook_sidebar_open } else { self.cfg.sidebar_collapsed };
            egui::Panel::left("sidebar")
                .exact_size(if collapsed { 60.0 } else { 240.0 })
                .frame(egui::Frame::NONE.fill(p.sidebar))
                .show_separator_line(false)
                .resizable(false)
                .show(ui, |ui| if collapsed { sidebar_collapsed(self, ui) } else { sidebar(self, ui) });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(p.window)).show(ui, |ui| match self.screen {
            Screen::Dashboard => dashboard::ui(self, ui),
            Screen::Stats => stats_view::ui(self, ui, None),
            Screen::Browser => browser::ui(self, ui),
            Screen::Subject(sid, tab) => subject::ui(self, ui, sid, tab),
            Screen::Review => review::ui(self, ui),
            Screen::Preset(id) => preset::ui(self, ui, id),
            Screen::Settings => settings::ui(self, ui),
        });

        note_editor::show(self, &ctx);
        modals::show(self, &ctx);
        palette::show(self, &ctx);
        toasts(self, &ctx);
        if !MAC {
            resize_edges(&ctx);
        }
        if !MAC && !ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            let r = ctx.content_rect();
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("outline"))).rect_stroke(r, 0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        }
        if let Some(pe) = self.page.as_mut() {
            pe.autosave(&self.db);
        }
        // Cartões em aprendizado vencem com o tempo; as contagens e a sessão acompanham.
        ctx.request_repaint_after(Duration::from_secs(if review { 1 } else { 5 }));
    }

    fn on_exit(&mut self) {
        if let Some(p) = self.page.as_mut() {
            p.flush(&self.db);
            p.snapshot(&self.db);
        }
        self.maybe_backup(true);
    }
}

fn toasts(app: &mut App, ctx: &egui::Context) {
    app.toasts.retain(|t| t.2.elapsed() < Duration::from_secs(if t.1 { 7 } else { 4 }));
    if app.toasts.is_empty() {
        return;
    }
    let p = pal();
    let screen = ctx.content_rect();
    let mut y = screen.bottom() - 28.0;
    for (i, (msg, err, _)) in app.toasts.iter().enumerate().rev() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique(("toast", i))));
        let g = painter.layout(msg.clone(), serif(13.0, W::Regular), if *err { Color32::WHITE } else { p.window }, 520.0);
        let size = g.size() + vec2(32.0, 20.0);
        let rect = Align2::CENTER_BOTTOM.anchor_size(pos2(screen.center().x, y), size);
        painter.rect_filled(rect, CornerRadius::same(9), if *err { p.again } else { p.text });
        painter.galley(rect.min + vec2(16.0, 10.0), g, p.window);
        y = rect.top() - 8.0;
    }
    ctx.request_repaint_after(Duration::from_millis(250));
}
