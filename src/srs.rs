//! Agendamento por repetição espaçada.
//!
//! O modelo de memória é o FSRS-6 da crate `fsrs` (a mesma do Anki); este
//! módulo acrescenta o que fica em volta dele, como no Anki: passos de
//! (re)aprendizado, graduação, ordem dos intervalos entre os botões, fuzz,
//! balanceamento de carga, intervalo máximo e o SM-2 clássico como
//! alternativa. É puro (sem banco nem GUI) para poder ser testado.

use chrono::{Local, NaiveDate, TimeZone};
use fsrs::{FSRS, MemoryState, NextStates};
use serde::{Deserialize, Serialize};

pub const MINUTE_MS: i64 = 60_000;
pub const DAY_MS: i64 = 86_400_000;
/// Cartões em aprendizado que vencem em até 20 min são mostrados antes da hora
/// quando não há mais nada na fila (igual ao "learn ahead limit" do Anki).
pub const LEARN_AHEAD_MS: i64 = 20 * MINUTE_MS;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum State {
    #[default]
    New = 0,
    Learning = 1,
    Review = 2,
    Relearning = 3,
}

impl State {
    pub fn from_i64(v: i64) -> State {
        match v {
            1 => State::Learning,
            2 => State::Review,
            3 => State::Relearning,
            _ => State::New,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            State::New => "novo",
            State::Learning => "aprendendo",
            State::Review => "revisão",
            State::Relearning => "reaprendendo",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Queue {
    Normal = 0,
    Suspended = -1,
    BuriedSibling = -2,
    BuriedUser = -3,
}

impl Queue {
    pub fn from_i64(v: i64) -> Queue {
        match v {
            -1 => Queue::Suspended,
            -2 => Queue::BuriedSibling,
            -3 => Queue::BuriedUser,
            _ => Queue::Normal,
        }
    }
}

/// O que o revlog registra sobre o tipo de revisão.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewKind {
    Learning = 0,
    Review = 1,
    Relearning = 2,
    Filtered = 3,
    Manual = 4,
}

impl ReviewKind {
    pub fn from_i64(v: i64) -> ReviewKind {
        match v {
            1 => ReviewKind::Review,
            2 => ReviewKind::Relearning,
            3 => ReviewKind::Filtered,
            4 => ReviewKind::Manual,
            _ => ReviewKind::Learning,
        }
    }
}

/// Estado de agendamento de um cartão (as colunas correspondentes de `cards`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sched {
    pub state: State,
    pub due: i64,
    /// Índice do passo atual de (re)aprendizado.
    pub step: u32,
    pub stability: Option<f32>,
    pub difficulty: Option<f32>,
    /// Fator de facilidade do SM-2 (2,5 = 250%).
    pub ease: f32,
    /// Intervalo atual em dias.
    pub interval: f32,
    pub reps: u32,
    pub lapses: u32,
    pub last_review: Option<i64>,
}

impl Sched {
    pub fn memory(&self) -> Option<MemoryState> {
        match (self.stability, self.difficulty) {
            (Some(s), Some(d)) if s > 0.0 => Some(MemoryState { stability: s, difficulty: d }),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Algorithm {
    #[default]
    Fsrs,
    Sm2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum ReviewOrder {
    /// Menor recuperabilidade primeiro (o mais "em risco").
    #[default]
    Retrievability,
    Due,
    Random,
    Deck,
}

impl ReviewOrder {
    pub const ALL: [ReviewOrder; 4] = [ReviewOrder::Retrievability, ReviewOrder::Due, ReviewOrder::Random, ReviewOrder::Deck];
    pub fn label(self) -> &'static str {
        match self {
            ReviewOrder::Retrievability => "Recuperabilidade ↑",
            ReviewOrder::Due => "Vencimento",
            ReviewOrder::Random => "Aleatória",
            ReviewOrder::Deck => "Por baralho",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum NewOrder {
    #[default]
    Created,
    Random,
}

impl NewOrder {
    pub const ALL: [NewOrder; 2] = [NewOrder::Created, NewOrder::Random];
    pub fn label(self) -> &'static str {
        match self {
            NewOrder::Created => "Ordem de criação",
            NewOrder::Random => "Aleatória",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum NewMix {
    #[default]
    Mixed,
    Before,
    After,
}

impl NewMix {
    pub const ALL: [NewMix; 3] = [NewMix::Mixed, NewMix::Before, NewMix::After];
    pub fn label(self) -> &'static str {
        match self {
            NewMix::Mixed => "Intercalados",
            NewMix::Before => "Antes das revisões",
            NewMix::After => "Depois das revisões",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum LeechAction {
    #[default]
    Tag,
    Suspend,
}

/// Opções de um preset (compartilhado por vários baralhos).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetConfig {
    pub algorithm: Algorithm,
    pub desired_retention: f32,
    /// Em minutos.
    pub learning_steps: Vec<f32>,
    pub relearning_steps: Vec<f32>,
    pub new_per_day: u32,
    pub reviews_per_day: u32,
    pub max_interval: u32,
    pub leech_threshold: u32,
    pub leech_action: LeechAction,
    pub bury_siblings: bool,
    pub new_order: NewOrder,
    pub review_order: ReviewOrder,
    pub new_mix: NewMix,
    pub fuzz: bool,
    pub load_balance: bool,
    /// SM-2: facilidade inicial, bônus do Fácil e multiplicador do Difícil.
    pub starting_ease: f32,
    pub easy_bonus: f32,
    pub hard_multiplier: f32,
}

impl Default for PresetConfig {
    fn default() -> Self {
        PresetConfig {
            algorithm: Algorithm::Fsrs,
            desired_retention: 0.9,
            learning_steps: vec![1.0, 10.0],
            relearning_steps: vec![10.0],
            new_per_day: 20,
            reviews_per_day: 200,
            max_interval: 36_500,
            leech_threshold: 8,
            leech_action: LeechAction::Tag,
            bury_siblings: true,
            new_order: NewOrder::Created,
            review_order: ReviewOrder::Retrievability,
            new_mix: NewMix::Mixed,
            fuzz: true,
            load_balance: true,
            starting_ease: 2.5,
            easy_bonus: 1.3,
            hard_multiplier: 1.2,
        }
    }
}

/// "1m 10m 1h 1d" ⇄ minutos.
pub fn parse_steps(s: &str) -> Option<Vec<f32>> {
    s.split_whitespace()
        .map(|t| {
            let t = t.trim().to_lowercase();
            let (num, mult) = if let Some(n) = t.strip_suffix('d') {
                (n, 1440.0)
            } else if let Some(n) = t.strip_suffix('h') {
                (n, 60.0)
            } else if let Some(n) = t.strip_suffix('s') {
                (n, 1.0 / 60.0)
            } else if let Some(n) = t.strip_suffix('m') {
                (n, 1.0)
            } else {
                (t.as_str(), 1.0)
            };
            num.replace(',', ".").parse::<f32>().ok().filter(|v| *v > 0.0).map(|v| v * mult)
        })
        .collect()
}

pub fn format_steps(steps: &[f32]) -> String {
    steps
        .iter()
        .map(|m| {
            if *m >= 1440.0 && (m / 1440.0).fract() == 0.0 {
                format!("{}d", m / 1440.0)
            } else if *m >= 60.0 && (m / 60.0).fract() == 0.0 {
                format!("{}h", m / 60.0)
            } else if *m < 1.0 {
                format!("{}s", (m * 60.0).round())
            } else {
                format!("{}m", trim_float(*m))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn trim_float(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Relógio de "dias de estudo": o dia vira na hora configurada (padrão 04:00),
/// não à meia-noite.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub rollover_hour: u32,
}

impl Clock {
    pub fn day_of(&self, ms: i64) -> i64 {
        let t = Local.timestamp_millis_opt(ms).single().unwrap_or_else(Local::now);
        let shifted = t - chrono::Duration::hours(self.rollover_hour as i64);
        shifted.date_naive().signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days()
    }

    /// Instante em que o dia de estudo `day` começa.
    pub fn day_start(&self, day: i64) -> i64 {
        let date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(day);
        let naive = date.and_hms_opt(self.rollover_hour.min(23), 0, 0).unwrap();
        Local.from_local_datetime(&naive).earliest().map(|t| t.timestamp_millis()).unwrap_or(day * DAY_MS)
    }

    pub fn today(&self) -> i64 {
        self.day_of(now_ms())
    }

    /// Fim do dia de estudo atual: tudo que vence antes disso é "para hoje".
    pub fn today_end(&self) -> i64 {
        self.day_start(self.today() + 1)
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Resultado de responder um cartão com uma nota.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub sched: Sched,
    /// Dias desde a revisão anterior (fracionário).
    pub elapsed_days: f32,
    /// Intervalo que estava agendado (dias).
    pub scheduled_days: f32,
    /// R prevista no momento da resposta.
    pub retrievability: Option<f32>,
    pub kind: ReviewKind,
    /// Virou sanguessuga nesta resposta.
    pub became_leech: bool,
}

impl Outcome {
    /// Atraso até a próxima revisão, em ms (para o rótulo do botão).
    pub fn delay_ms(&self, now: i64) -> i64 {
        (self.sched.due - now).max(0)
    }
}

/// Tudo que o agendador precisa além do cartão.
pub struct Ctx<'a> {
    pub cfg: &'a PresetConfig,
    pub fsrs: &'a FSRS,
    /// Decaimento (w20) para calcular R.
    pub decay: f32,
    pub clock: Clock,
    pub now: i64,
    /// Semente determinística (id do cartão): a pré-visualização dos botões
    /// coincide com o que é aplicado.
    pub seed: u64,
    /// Quantos cartões já vencem em cada dia (para o balanceamento de carga).
    pub due_on_day: &'a dyn Fn(i64) -> usize,
    /// Revisão fora da fila normal (sessão filtrada / antecipada).
    pub filtered: bool,
}

pub fn retrievability(sched: &Sched, decay: f32, clock: &Clock, now: i64) -> Option<f32> {
    let mem = sched.memory()?;
    let last = sched.last_review?;
    let _ = clock;
    let days = ((now - last) as f32 / DAY_MS as f32).max(0.0);
    Some(fsrs::current_retrievability(mem, days, decay))
}

/// Os quatro resultados possíveis (Errei, Difícil, Bom, Fácil).
pub fn preview(card: &Sched, ctx: &Ctx) -> [Outcome; 4] {
    [answer(card, 1, ctx), answer(card, 2, ctx), answer(card, 3, ctx), answer(card, 4, ctx)]
}

pub fn answer(card: &Sched, rating: u32, ctx: &Ctx) -> Outcome {
    let cfg = ctx.cfg;
    let today = ctx.clock.day_of(ctx.now);
    let elapsed_days_whole = card.last_review.map(|l| (today - ctx.clock.day_of(l)).max(0)).unwrap_or(0) as u32;
    let elapsed_days = card.last_review.map(|l| ((ctx.now - l) as f32 / DAY_MS as f32).max(0.0)).unwrap_or(0.0);
    let r = retrievability(card, ctx.decay, &ctx.clock, ctx.now);

    // O FSRS acompanha a memória em todas as respostas, inclusive no aprendizado.
    let next: Option<NextStates> = ctx.fsrs.next_states(card.memory(), cfg.desired_retention, elapsed_days_whole).ok();
    let pick = |n: &NextStates, g: u32| match g {
        1 => n.again.clone(),
        2 => n.hard.clone(),
        3 => n.good.clone(),
        _ => n.easy.clone(),
    };

    let mut s = card.clone();
    s.reps += 1;
    s.last_review = Some(ctx.now);
    if let Some(n) = &next {
        let st = pick(n, rating);
        s.stability = Some(st.memory.stability);
        s.difficulty = Some(st.memory.difficulty);
    }
    if s.ease <= 0.0 {
        s.ease = cfg.starting_ease;
    }

    let kind = if ctx.filtered {
        ReviewKind::Filtered
    } else {
        match card.state {
            State::New | State::Learning => ReviewKind::Learning,
            State::Review => ReviewKind::Review,
            State::Relearning => ReviewKind::Relearning,
        }
    };
    let scheduled_days = card.interval;
    let mut became_leech = false;

    // Intervalos (dias) que cada botão daria se o cartão graduasse/revisasse agora.
    let fsrs_ivls = next.as_ref().map(|n| [n.again.interval, n.hard.interval, n.good.interval, n.easy.interval]);

    match card.state {
        State::New | State::Learning => {
            let steps = &cfg.learning_steps;
            let i = if card.state == State::New { 0 } else { card.step as usize };
            learning_step(&mut s, rating, i, steps, State::Learning, ctx, fsrs_ivls, elapsed_days_whole, card);
        }
        State::Relearning => {
            let steps = &cfg.relearning_steps;
            learning_step(&mut s, rating, card.step as usize, steps, State::Relearning, ctx, fsrs_ivls, elapsed_days_whole, card);
        }
        State::Review => {
            if rating == 1 {
                s.lapses += 1;
                if cfg.leech_threshold > 0 && s.lapses >= cfg.leech_threshold {
                    let half = (cfg.leech_threshold / 2).max(1);
                    if (s.lapses - cfg.leech_threshold) % half == 0 {
                        became_leech = true;
                    }
                }
                let lapse_ivl = match cfg.algorithm {
                    Algorithm::Fsrs => fsrs_ivls.map(|v| v[0]).unwrap_or(1.0),
                    Algorithm::Sm2 => {
                        s.ease = (s.ease - 0.2).max(1.3);
                        1.0
                    }
                };
                s.interval = lapse_ivl.round().clamp(1.0, cfg.max_interval as f32);
                if cfg.relearning_steps.is_empty() {
                    s.state = State::Review;
                    s.step = 0;
                    s.due = review_due(s.interval as i64, ctx);
                } else {
                    s.state = State::Relearning;
                    s.step = 0;
                    s.due = ctx.now + (cfg.relearning_steps[0] * MINUTE_MS as f32) as i64;
                }
            } else {
                let ivls = review_intervals(card, ctx, fsrs_ivls, elapsed_days_whole as f32);
                let ivl = ivls[rating as usize - 2];
                if cfg.algorithm == Algorithm::Sm2 {
                    match rating {
                        2 => s.ease = (s.ease - 0.15).max(1.3),
                        4 => s.ease += 0.15,
                        _ => {}
                    }
                }
                let ivl = fuzzed(ivl, elapsed_days_whole as f32, ctx);
                s.interval = ivl as f32;
                s.state = State::Review;
                s.step = 0;
                s.due = review_due(ivl, ctx);
            }
        }
    }

    Outcome { sched: s, elapsed_days, scheduled_days, retrievability: r, kind, became_leech }
}

#[allow(clippy::too_many_arguments)]
fn learning_step(
    s: &mut Sched,
    rating: u32,
    i: usize,
    steps: &[f32],
    learning_state: State,
    ctx: &Ctx,
    fsrs_ivls: Option<[f32; 4]>,
    elapsed: u32,
    card: &Sched,
) {
    let graduate = |s: &mut Sched, easy: bool| {
        let ivl = graduating_interval(card, easy, ctx, fsrs_ivls, learning_state);
        let ivl = fuzzed(ivl, elapsed as f32, ctx);
        s.interval = ivl as f32;
        s.state = State::Review;
        s.step = 0;
        s.due = review_due(ivl, ctx);
    };
    let delay = |m: f32| (m * MINUTE_MS as f32) as i64;
    if steps.is_empty() {
        match rating {
            4 => graduate(s, true),
            _ => graduate(s, false),
        }
        return;
    }
    let i = i.min(steps.len() - 1);
    match rating {
        1 => {
            s.state = learning_state;
            s.step = 0;
            s.due = ctx.now + delay(steps[0]);
        }
        2 => {
            s.state = learning_state;
            s.step = i as u32;
            let m = if i == 0 {
                if steps.len() > 1 { (steps[0] + steps[1]) / 2.0 } else { (steps[0] * 1.5).min(steps[0] + 1440.0) }
            } else {
                steps[i]
            };
            s.due = ctx.now + delay(m);
        }
        3 => {
            if i + 1 < steps.len() {
                s.state = learning_state;
                s.step = (i + 1) as u32;
                s.due = ctx.now + delay(steps[i + 1]);
            } else {
                graduate(s, false);
            }
        }
        _ => graduate(s, true),
    }
}

fn graduating_interval(card: &Sched, easy: bool, ctx: &Ctx, fsrs_ivls: Option<[f32; 4]>, learning_state: State) -> i64 {
    let cfg = ctx.cfg;
    let max = cfg.max_interval.max(1) as f32;
    match cfg.algorithm {
        Algorithm::Fsrs => {
            let v = fsrs_ivls.unwrap_or([1.0, 1.0, 1.0, 4.0]);
            let good = v[2].round().clamp(1.0, max);
            if easy { v[3].round().max(good + 1.0).clamp(1.0, max) as i64 } else { good as i64 }
        }
        Algorithm::Sm2 => {
            if learning_state == State::Relearning {
                // Volta ao intervalo do lapso (já gravado em `interval`).
                card.interval.max(1.0).min(max) as i64 + if easy { 1 } else { 0 }
            } else if easy {
                4.0f32.min(max) as i64
            } else {
                1
            }
        }
    }
}

/// [Difícil, Bom, Fácil] em dias, já ordenados e limitados.
fn review_intervals(card: &Sched, ctx: &Ctx, fsrs_ivls: Option<[f32; 4]>, elapsed: f32) -> [i64; 3] {
    let cfg = ctx.cfg;
    let max = cfg.max_interval.max(1) as f32;
    let (mut hard, mut good, mut easy) = match cfg.algorithm {
        Algorithm::Fsrs => {
            let v = fsrs_ivls.unwrap_or([1.0, 1.0, 2.0, 4.0]);
            (v[1].round(), v[2].round(), v[3].round())
        }
        Algorithm::Sm2 => {
            let cur = card.interval.max(1.0);
            let late = (elapsed - cur).max(0.0);
            let hard = cur * cfg.hard_multiplier;
            let good = (cur + late / 2.0) * card.ease.max(1.3);
            let easy = (cur + late) * card.ease.max(1.3) * cfg.easy_bonus;
            (hard.round(), good.round(), easy.round())
        }
    };
    hard = hard.clamp(1.0, max);
    if cfg.algorithm == Algorithm::Fsrs {
        hard = hard.min(good.max(1.0));
    }
    good = good.max(hard + 1.0).clamp(1.0, max);
    easy = easy.max(good + 1.0).clamp(1.0, max);
    [hard as i64, good as i64, easy as i64]
}

fn review_due(ivl_days: i64, ctx: &Ctx) -> i64 {
    let today = ctx.clock.day_of(ctx.now);
    ctx.clock.day_start(today + ivl_days.max(1))
}

/// Faixa de fuzz do Anki: nada abaixo de 2,5 d; 15% até 7 d, 10% até 20 d, 5% acima.
pub fn fuzz_range(interval: f32, elapsed_days: f32, max: u32) -> (i64, i64) {
    if interval < 2.5 {
        let i = interval.round() as i64;
        return (i, i);
    }
    let mut delta = 1.0;
    for (start, end, factor) in [(2.5f32, 7.0f32, 0.15f32), (7.0, 20.0, 0.1), (20.0, f32::MAX, 0.05)] {
        delta += factor * (interval.min(end) - start).max(0.0);
    }
    let mut lo = (interval - delta).round().max(2.0) as i64;
    let hi = (interval + delta).round() as i64;
    if interval > elapsed_days {
        lo = lo.max(elapsed_days as i64 + 1);
    }
    let max = max.max(1) as i64;
    (lo.min(max).min(hi), hi.min(max))
}

fn fuzzed(ivl: i64, elapsed: f32, ctx: &Ctx) -> i64 {
    let cfg = ctx.cfg;
    if !cfg.fuzz {
        return ivl.max(1);
    }
    let (lo, hi) = fuzz_range(ivl as f32, elapsed, cfg.max_interval);
    if lo >= hi {
        return lo.max(1);
    }
    let today = ctx.clock.day_of(ctx.now);
    let mut rng = SplitMix(ctx.seed ^ (ivl as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    if cfg.load_balance {
        // Escolhe o dia mais leve da faixa; empates (e pesos parecidos) pela semente.
        let mut best = lo;
        let mut best_w = f64::MAX;
        for d in lo..=hi {
            let n = (ctx.due_on_day)(today + d) as f64;
            // Leve preferência pelo intervalo original, como no Anki.
            let w = n + (d - ivl).abs() as f64 * 0.5 + rng.next_f64() * 0.9;
            if w < best_w {
                best_w = w;
                best = d;
            }
        }
        best.max(1)
    } else {
        lo + (rng.next_u64() % (hi - lo + 1) as u64) as i64
    }
}

struct SplitMix(u64);

impl SplitMix {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// "10 min", "3 d", "2,1 meses", "1,4 anos".
pub fn interval_label(ms: i64) -> String {
    let mins = ms as f64 / MINUTE_MS as f64;
    if mins < 1.0 {
        return "< 1 min".into();
    }
    if mins < 60.0 {
        return format!("{} min", mins.round());
    }
    let hours = mins / 60.0;
    if hours < 24.0 {
        return format!("{} h", decimal(hours, 1));
    }
    let days = hours / 24.0;
    days_label(days)
}

pub fn days_label(days: f64) -> String {
    if days < 1.0 {
        return format!("{} h", decimal(days * 24.0, 1));
    }
    if days < 30.0 {
        return format!("{} d", decimal(days.round(), 0));
    }
    if days < 365.0 {
        let m = days / 30.4;
        return format!("{} {}", decimal(m, 1), if m < 1.95 { "mês" } else { "meses" });
    }
    let y = days / 365.0;
    format!("{} {}", decimal(y, 1), if y < 1.95 { "ano" } else { "anos" })
}

/// Número com vírgula decimal e sem ",0" sobrando.
pub fn decimal(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    s.replace('.', ",")
}

/// Instância do FSRS para um preset (parâmetros vazios = padrão).
pub fn fsrs_for(params: &[f32]) -> (FSRS, f32) {
    let f = FSRS::new(params).unwrap_or_else(|_| FSRS::new(&[]).expect("parâmetros padrão"));
    let decay = if params.len() >= 21 { params[20] } else { fsrs::FSRS6_DEFAULT_DECAY };
    (f, decay)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(cfg: &'a PresetConfig, f: &'a FSRS, now: i64, due: &'a dyn Fn(i64) -> usize) -> Ctx<'a> {
        Ctx { cfg, fsrs: f, decay: fsrs::FSRS6_DEFAULT_DECAY, clock: Clock { rollover_hour: 4 }, now, seed: 42, due_on_day: due, filtered: false }
    }

    #[test]
    fn new_card_walks_learning_steps_then_graduates() {
        let cfg = PresetConfig::default();
        let (f, _) = fsrs_for(&[]);
        let zero = |_: i64| 0usize;
        let now = 1_760_000_000_000;
        let c = ctx(&cfg, &f, now, &zero);
        let card = Sched::default();

        let again = answer(&card, 1, &c);
        assert_eq!(again.sched.state, State::Learning);
        assert_eq!(again.sched.due - now, MINUTE_MS);

        let good = answer(&card, 3, &c);
        assert_eq!(good.sched.state, State::Learning);
        assert_eq!(good.sched.step, 1);
        assert_eq!(good.sched.due - now, 10 * MINUTE_MS);

        let hard = answer(&card, 2, &c);
        assert_eq!(hard.sched.due - now, (5.5 * MINUTE_MS as f32) as i64);

        let easy = answer(&card, 4, &c);
        assert_eq!(easy.sched.state, State::Review);
        assert!(easy.sched.interval >= 2.0);

        // Segundo Bom no último passo gradua.
        let c2 = ctx(&cfg, &f, now + 10 * MINUTE_MS, &zero);
        let grad = answer(&good.sched, 3, &c2);
        assert_eq!(grad.sched.state, State::Review);
        assert!(grad.sched.interval >= 1.0);
        assert!(grad.sched.stability.unwrap() > 0.0);
    }

    #[test]
    fn review_buttons_are_ordered_and_lapse_relearns() {
        let cfg = PresetConfig::default();
        let (f, _) = fsrs_for(&[]);
        let zero = |_: i64| 0usize;
        let now = 1_760_000_000_000;
        let card = Sched {
            state: State::Review,
            due: now,
            stability: Some(10.0),
            difficulty: Some(5.0),
            ease: 2.5,
            interval: 10.0,
            reps: 5,
            last_review: Some(now - 10 * DAY_MS),
            ..Default::default()
        };
        let c = ctx(&cfg, &f, now, &zero);
        let p = preview(&card, &c);
        assert_eq!(p[0].sched.state, State::Relearning);
        assert_eq!(p[0].sched.lapses, 1);
        let ivl: Vec<f32> = p[1..].iter().map(|o| o.sched.interval).collect();
        assert!(ivl[0] < ivl[1] && ivl[1] < ivl[2], "{ivl:?}");
        assert!(p[0].retrievability.unwrap() > 0.85 && p[0].retrievability.unwrap() < 0.95);
    }

    #[test]
    fn sm2_grows_by_ease() {
        let cfg = PresetConfig { algorithm: Algorithm::Sm2, fuzz: false, ..Default::default() };
        let (f, _) = fsrs_for(&[]);
        let zero = |_: i64| 0usize;
        let now = 1_760_000_000_000;
        let card = Sched { state: State::Review, interval: 10.0, ease: 2.5, last_review: Some(now - 10 * DAY_MS), ..Default::default() };
        let c = ctx(&cfg, &f, now, &zero);
        let good = answer(&card, 3, &c);
        assert_eq!(good.sched.interval, 25.0);
        let hard = answer(&card, 2, &c);
        assert_eq!(hard.sched.interval, 12.0);
        assert!((hard.sched.ease - 2.35).abs() < 1e-4);
    }

    #[test]
    fn fuzz_range_matches_anki() {
        assert_eq!(fuzz_range(1.0, 0.0, 36500), (1, 1));
        let (lo, hi) = fuzz_range(10.0, 5.0, 36500);
        assert!(lo <= 9 && hi >= 11, "{lo} {hi}");
        assert_eq!(fuzz_range(100.0, 50.0, 36500).1 - 100, 100 - fuzz_range(100.0, 50.0, 36500).0);
    }

    #[test]
    fn steps_roundtrip() {
        assert_eq!(parse_steps("1m 10m 1h 1d").unwrap(), vec![1.0, 10.0, 60.0, 1440.0]);
        assert_eq!(format_steps(&[1.0, 10.0, 60.0, 1440.0]), "1m 10m 1h 1d");
        assert!(parse_steps("1x").is_none());
    }

    #[test]
    fn labels() {
        assert_eq!(interval_label(10 * MINUTE_MS), "10 min");
        assert_eq!(days_label(3.0), "3 d");
        assert_eq!(days_label(64.0), "2,1 meses");
    }
}
