//! Cores, fontes e textos do design (`theme.css` do canvas "Discite —
//! Interface"): tema escuro "grafite" e claro "papel", tudo em Literata com
//! JetBrains Mono para teclas e código.

use eframe::egui::{self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect, Stroke, text::LayoutJob};
use crate::{tr, tr_n};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub titlebar: Color32,
    pub sidebar: Color32,
    pub input: Color32,
    pub window: Color32,
    pub card: Color32,
    pub chip: Color32,
    pub selected: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub border_control: Color32,
    pub border_strong: Color32,
    pub line: Color32,
    pub line_row: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub muted: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub accent_tint: Color32,
    pub on_accent: Color32,
    pub again: Color32,
    pub hard: Color32,
    pub good: Color32,
    pub easy: Color32,
    pub mark: Color32,
    pub heat: [Color32; 5],
    pub subjects: [Color32; 8],
    pub scrim: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// `rgba(r,g,b,a)` do CSS, pré-multiplicado como o egui espera.
fn rgba(v: u32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied((v >> 16) as u8, (v >> 8) as u8, v as u8, (a * 255.0).round() as u8)
}

pub fn dark() -> Pal {
    Pal {
        dark: true,
        titlebar: hex(0x0C0D10),
        sidebar: hex(0x0E0F12),
        input: hex(0x0E0F12),
        window: hex(0x111216),
        card: hex(0x16181C),
        chip: hex(0x1E2025),
        selected: hex(0x24272D),
        hover: hex(0x1A1C21),
        border: hex(0x25272D),
        border_control: hex(0x2F323A),
        border_strong: hex(0x3A3E47),
        line: hex(0x1D1F24),
        line_row: hex(0x1A1C20),
        text: hex(0xECEDEF),
        text2: hex(0xC8CAD0),
        muted: hex(0x9A9DA6),
        dim: hex(0x858893),
        accent: hex(0x8E9CFF),
        accent_hover: hex(0xA9B3FF),
        accent_soft: rgba(0x8E9CFF, 0.14),
        accent_tint: rgba(0x8E9CFF, 0.07),
        on_accent: hex(0x0D0F1F),
        again: hex(0xF2666A),
        hard: hex(0xF0A63E),
        good: hex(0x4FC48C),
        easy: hex(0x5AAEFF),
        mark: rgba(0xF0A63E, 0.22),
        heat: [hex(0x1A1C21), hex(0x2A3060), hex(0x3F4A9A), hex(0x6474D6), hex(0x9AA6FF)],
        subjects: [
            hex(0xEE8064),
            hex(0x52BFAE),
            hex(0xA88CF0),
            hex(0xD4AF4C),
            hex(0x6FA8F0),
            hex(0xE77FB0),
            hex(0x8CC461),
            hex(0xE09A5A),
        ],
        scrim: Color32::from_black_alpha(140),
    }
}

pub fn light() -> Pal {
    Pal {
        dark: false,
        titlebar: hex(0xE9E5D9),
        sidebar: hex(0xF0EDE3),
        input: hex(0xFCFBF7),
        window: hex(0xF6F4EC),
        card: hex(0xFBFAF5),
        chip: hex(0xECE8DD),
        selected: hex(0xE3DED0),
        hover: hex(0xF0EDE4),
        border: hex(0xDFDACD),
        border_control: hex(0xCEC8B8),
        border_strong: hex(0xB7B09E),
        line: hex(0xE6E2D6),
        line_row: hex(0xECE9DF),
        text: hex(0x2A2822),
        text2: hex(0x3E3B33),
        muted: hex(0x5C584D),
        dim: hex(0x6A665A),
        accent: hex(0x3445A6),
        accent_hover: hex(0x27378C),
        accent_soft: rgba(0x3445A6, 0.10),
        accent_tint: rgba(0x3445A6, 0.045),
        on_accent: hex(0xFCFBF7),
        again: hex(0xB8332D),
        hard: hex(0x8F5A08),
        good: hex(0x21744A),
        easy: hex(0x235FB0),
        mark: rgba(0xDEAA28, 0.30),
        heat: [hex(0xE9E5DA), hex(0xC8C9D9), hex(0x9AA0D2), hex(0x5F6CC0), hex(0x3445A6)],
        subjects: [
            hex(0xC2502F),
            hex(0x1F8574),
            hex(0x6F4FC2),
            hex(0x9C7613),
            hex(0x2D6CB5),
            hex(0xB23E7A),
            hex(0x4E7F23),
            hex(0xB06420),
        ],
        scrim: Color32::from_rgba_unmultiplied(8, 8, 6, 122),
    }
}

static DARK: AtomicBool = AtomicBool::new(true);

pub fn pal() -> Pal {
    if DARK.load(Ordering::Relaxed) { dark() } else { light() }
}

pub fn subject_color(i: usize) -> Color32 {
    let p = pal();
    p.subjects[i % p.subjects.len()]
}

/// Fundo translúcido de uma cor (os `--again-soft` etc. do CSS).
pub fn soft(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8)
}

#[derive(Clone, Copy, PartialEq)]
pub enum W {
    Regular,
    Medium,
    Semi,
    Bold,
    Italic,
    SemiItalic,
}

pub fn serif(size: f32, w: W) -> FontId {
    let fam = match w {
        W::Regular => FontFamily::Proportional,
        W::Medium => FontFamily::Name("medium".into()),
        W::Semi => FontFamily::Name("semi".into()),
        W::Bold => FontFamily::Name("bold".into()),
        W::Italic => FontFamily::Name("italic".into()),
        W::SemiItalic => FontFamily::Name("semi-italic".into()),
    };
    FontId::new(size, fam)
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, data: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
    };
    add(&mut fonts, "lit", include_bytes!("../../assets/fonts/Literata-Regular.ttf"));
    add(&mut fonts, "lit-medium", include_bytes!("../../assets/fonts/Literata-Medium.ttf"));
    add(&mut fonts, "lit-semi", include_bytes!("../../assets/fonts/Literata-SemiBold.ttf"));
    add(&mut fonts, "lit-bold", include_bytes!("../../assets/fonts/Literata-Bold.ttf"));
    add(&mut fonts, "lit-italic", include_bytes!("../../assets/fonts/Literata-Italic.ttf"));
    add(&mut fonts, "lit-semi-italic", include_bytes!("../../assets/fonts/Literata-SemiBoldItalic.ttf"));
    add(&mut fonts, "jb-mono", include_bytes!("../../assets/fonts/JetBrainsMono-400.ttf"));

    // As fontes do egui ficam de reserva para símbolos (▾ › ◐ ⇧ ∑…). A Hack (a
    // monoespaçada do egui) cobre setas e formas geométricas; sem ela esses
    // caracteres dependeriam das fontes do sistema, que no macOS não os têm.
    let mut fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if !fallback.iter().any(|f| f == "Hack") {
        fallback.push("Hack".to_owned());
    }
    let family = |first: &str| {
        let mut v = vec![first.to_owned()];
        v.extend(fallback.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, family("lit"));
    for (fam, font) in [("medium", "lit-medium"), ("semi", "lit-semi"), ("bold", "lit-bold"), ("italic", "lit-italic"), ("semi-italic", "lit-semi-italic")] {
        fonts.families.insert(FontFamily::Name(fam.into()), family(font));
    }
    let mut mono_fam = vec!["jb-mono".to_owned()];
    mono_fam.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Monospace, mono_fam);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, dark_mode: bool) {
    DARK.store(dark_mode, Ordering::Relaxed);
    let p = pal();
    let mut v = if dark_mode { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.window;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = egui::CornerRadius::same(14);
    v.menu_corner_radius = egui::CornerRadius::same(10);
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.chip;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = soft(p.accent, if dark_mode { 0.35 } else { 0.25 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(1.6, p.accent);
    v.popup_shadow = egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(if dark_mode { 120 } else { 40 }) };
    v.window_shadow = egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: Color32::from_black_alpha(if dark_mode { 140 } else { 46 }) };
    for (w, fill) in [
        (&mut v.widgets.noninteractive, p.card),
        (&mut v.widgets.inactive, p.chip),
        (&mut v.widgets.hovered, p.selected),
        (&mut v.widgets.active, p.selected),
        (&mut v.widgets.open, p.selected),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = egui::CornerRadius::same(8);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text2);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border_control);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text2);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    let theme = if dark_mode { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(if dark_mode { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
    ctx.set_visuals_of(theme, v);
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(egui::TextStyle::Body, serif(14.0, W::Regular));
        s.text_styles.insert(egui::TextStyle::Button, serif(13.0, W::Regular));
        s.text_styles.insert(egui::TextStyle::Small, serif(11.5, W::Regular));
        s.text_styles.insert(egui::TextStyle::Monospace, mono(12.5));
        s.text_styles.insert(egui::TextStyle::Heading, serif(22.0, W::Semi));
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        s.spacing.menu_margin = egui::Margin::same(6);
    });
}

/// No macOS a janela usa a barra de título nativa e os atalhos são com ⌘.
pub const MAC: bool = cfg!(target_os = "macos");

/// Troca "Ctrl" pelos símbolos do Mac nos textos de atalho ("Ctrl ⇧ A" → "⇧ ⌘ A").
/// Os atalhos em si já usam `Modifiers::COMMAND`, que é ⌘ no Mac.
pub fn keys(s: &str) -> std::borrow::Cow<'_, str> {
    if !MAC || !s.contains("Ctrl") {
        return std::borrow::Cow::Borrowed(s);
    }
    std::borrow::Cow::Owned(
        s.replace("Ctrl ⇧ ", "⇧ ⌘ ")
            .replace("Ctrl Shift ", "⇧ ⌘ ")
            .replace("Ctrl+Shift+", "⇧⌘")
            .replace("Ctrl+", "⌘")
            .replace("Ctrl ", "⌘ ")
            .replace("Ctrl", "⌘"),
    )
}

/// Uma linha cortada com "…" em `max_w`, com espaçamento entre letras opcional.
pub fn line(painter: &Painter, s: &str, font: FontId, color: Color32, max_w: f32, spacing: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(keys(s).into_owned(), egui::TextFormat { font_id: font, color, extra_letter_spacing: spacing, ..Default::default() });
    job.wrap.max_width = max_w.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    painter.layout_job(job)
}

/// Pinta uma linha ancorada em `pos`; devolve o retângulo.
/// Lupa desenhada (o ⌕ não existe nas fontes embutidas).
pub fn search_icon(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let r = size * 0.3;
    let c = center + egui::vec2(-size * 0.08, -size * 0.08);
    let stroke = Stroke::new((size * 0.09).max(1.2), color);
    painter.circle_stroke(c, r, stroke);
    let d = std::f32::consts::FRAC_1_SQRT_2;
    painter.line_segment([c + egui::vec2(r * d, r * d), c + egui::vec2(r * d + size * 0.22, r * d + size * 0.22)], stroke);
}

/// "×" desenhado, para botões de fechar e remover.
pub fn close_icon(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let h = size * 0.3;
    let stroke = Stroke::new((size * 0.1).max(1.2), color);
    painter.line_segment([center + egui::vec2(-h, -h), center + egui::vec2(h, h)], stroke);
    painter.line_segment([center + egui::vec2(-h, h), center + egui::vec2(h, -h)], stroke);
}

/// Alça de arrastar: duas colunas de três pontos.
pub fn grip_icon(painter: &Painter, center: Pos2, color: Color32) {
    for x in [-2.5, 2.5] {
        for y in [-5.0, 0.0, 5.0] {
            painter.circle_filled(center + egui::vec2(x, y), 1.3, color);
        }
    }
}

pub fn text(painter: &Painter, pos: Pos2, align: Align2, s: &str, font: FontId, color: Color32, max_w: f32) -> Rect {
    let g = line(painter, s, font, color, max_w, 0.0);
    let rect = align.anchor_size(pos, g.size());
    painter.galley(rect.min, g, color);
    rect
}

/// O rótulo `.caps` do design: 10,5 px, caixa alta, `letter-spacing: .1em`.
pub fn caps(painter: &Painter, pos: Pos2, align: Align2, s: &str, color: Color32) -> Rect {
    caps_sized(painter, pos, align, s, 10.5, color)
}

pub fn caps_sized(painter: &Painter, pos: Pos2, align: Align2, s: &str, size: f32, color: Color32) -> Rect {
    let g = line(painter, &s.to_uppercase(), serif(size, W::Semi), color, f32::INFINITY, size * 0.1);
    let rect = align.anchor_size(pos, g.size());
    painter.galley(rect.min, g, color);
    rect
}

pub fn text_width(painter: &Painter, s: &str, font: FontId) -> f32 {
    painter.layout_no_wrap(s.to_owned(), font, Color32::WHITE).size().x
}

/// Inteiro com separador de milhar ("1.946").
/// Inteiro com o separador de milhar do idioma ("1,946" / "1.946").
pub fn thousands(n: i64) -> String {
    crate::i18n::thousands(n)
}

pub fn pct(v: f32, places: usize) -> String {
    format!("{}%", crate::srs::decimal(v as f64 * 100.0, places))
}

/// "2 h ago", "yesterday", "3 days ago".
pub fn ago(ms: i64) -> String {
    let now = crate::srs::now_ms();
    let mins = (now - ms).max(0) / 60_000;
    if mins < 1 {
        return tr!("time.now");
    }
    if mins < 60 {
        return tr!("time.minutes_ago", n = mins);
    }
    let hours = mins / 60;
    if hours < 24 {
        return tr!("time.hours_ago", n = hours);
    }
    let days = hours / 24;
    match days {
        1 => tr!("time.yesterday"),
        2..=29 => tr!("time.days_ago", n = days),
        _ => tr_n!("time.months_ago", days / 30),
    }
}

/// "Saturday, October 24" / "Sábado, 24 de outubro".
pub fn long_date() -> String {
    use chrono::Datelike;
    let t = chrono::Local::now();
    tr!(
        "date.long",
        weekday = tr!(WEEKDAYS[t.weekday().num_days_from_monday() as usize]),
        day = t.day(),
        month = tr!(MONTHS[t.month0() as usize])
    )
}

/// Chaves de tradução (o texto sai com `tr!`).
pub const MONTHS: [&str; 12] = ["month.jan", "month.feb", "month.mar", "month.apr", "month.may", "month.jun", "month.jul", "month.aug", "month.sep", "month.oct", "month.nov", "month.dec"];
pub const MONTHS_SHORT: [&str; 12] = ["month_short.jan", "month_short.feb", "month_short.mar", "month_short.apr", "month_short.may", "month_short.jun", "month_short.jul", "month_short.aug", "month_short.sep", "month_short.oct", "month_short.nov", "month_short.dec"];
pub const WEEKDAYS: [&str; 7] = ["weekday.mon", "weekday.tue", "weekday.wed", "weekday.thu", "weekday.fri", "weekday.sat", "weekday.sun"];
pub const WEEKDAYS_SHORT: [&str; 7] = ["weekday_short.mon", "weekday_short.tue", "weekday_short.wed", "weekday_short.thu", "weekday_short.fri", "weekday_short.sat", "weekday_short.sun"];

/// "Oct 24" / "24 out".
pub fn short_date(ms: i64) -> String {
    use chrono::{Datelike, TimeZone};
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| tr!("date.short", day = t.day(), month = tr!(MONTHS_SHORT[t.month0() as usize])))
        .unwrap_or_default()
}

pub fn greeting() -> String {
    use chrono::Timelike;
    tr!(match chrono::Local::now().hour() {
        5..=11 => "app.good_morning",
        12..=17 => "app.good_afternoon",
        _ => "app.good_evening",
    })
}
