//! Estatísticas calculadas a partir do revlog e dos cartões.

use crate::db::Db;
use crate::srs::{self, Clock, DAY_MS};
use anyhow::Result;
use std::collections::HashMap;

/// Filtro de escopo: `None` = tudo; `Some(ids)` = só esses baralhos.
pub struct Scope<'a>(pub Option<&'a [i64]>);

impl Scope<'_> {
    fn cards(&self, col: &str) -> String {
        match self.0 {
            None => String::new(),
            Some(ids) if ids.is_empty() => " AND 0".into(),
            Some(ids) => format!(" AND {col} IN (SELECT id FROM cards WHERE deck_id IN ({}))", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")),
        }
    }
    fn decks(&self) -> String {
        match self.0 {
            None => String::new(),
            Some(ids) if ids.is_empty() => " AND 0".into(),
            Some(ids) => format!(" AND deck_id IN ({})", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")),
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct Overview {
    pub reviews: u32,
    pub days: u32,
    /// Acerto (≠ Errei) em revisões de cartões maduros (intervalo ≥ 21 d).
    pub retention: Option<f32>,
    pub time_ms: i64,
    pub avg_secs: f32,
}

pub fn overview(db: &Db, scope: &Scope, since: i64, days: u32) -> Result<Overview> {
    let (n, t): (i64, i64) = db.conn.query_row(
        &format!("SELECT COUNT(*), COALESCE(SUM(duration_ms), 0) FROM revlog WHERE undone = 0 AND at >= ?1 AND kind != 4{}", scope.cards("card_id")),
        [since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (mn, mok): (i64, i64) = db.conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN rating > 1 THEN 1 ELSE 0 END), 0) FROM revlog
             WHERE undone = 0 AND at >= ?1 AND kind = 1 AND scheduled_days >= 21{}",
            scope.cards("card_id")
        ),
        [since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Sem revisões maduras no período, usa todas as revisões (kind = 1).
    let retention = if mn >= 10 {
        Some(mok as f32 / mn as f32)
    } else {
        let (an, aok): (i64, i64) = db.conn.query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(CASE WHEN rating > 1 THEN 1 ELSE 0 END), 0) FROM revlog WHERE undone = 0 AND at >= ?1 AND kind = 1{}", scope.cards("card_id")),
            [since],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if an > 0 { Some(aok as f32 / an as f32) } else { None }
    };
    Ok(Overview {
        reviews: n as u32,
        days,
        retention,
        time_ms: t,
        avg_secs: if n > 0 { t as f32 / n as f32 / 1000.0 } else { 0.0 },
    })
}

/// Revisões por dia de estudo (para o mapa de calor).
pub fn per_day(db: &Db, scope: &Scope, clock: &Clock, since: i64) -> Result<HashMap<i64, u32>> {
    let mut st = db.conn.prepare(&format!("SELECT at FROM revlog WHERE undone = 0 AND at >= ?1{}", scope.cards("card_id")))?;
    let mut out = HashMap::new();
    for at in st.query_map([since], |r| r.get::<_, i64>(0))? {
        *out.entry(clock.day_of(at?)).or_insert(0) += 1;
    }
    Ok(out)
}

/// Dias seguidos com revisões até hoje (ou até ontem, se hoje ainda não estudou).
pub fn streak(days: &HashMap<i64, u32>, today: i64) -> u32 {
    let mut d = if days.contains_key(&today) { today } else { today - 1 };
    let mut n = 0;
    while days.get(&d).copied().unwrap_or(0) > 0 {
        n += 1;
        d -= 1;
    }
    n
}

/// Por dia: [revisão, aprendizado, reaprendizado].
pub fn daily_by_kind(db: &Db, scope: &Scope, clock: &Clock, days: i64) -> Result<Vec<[u32; 3]>> {
    let today = clock.today();
    let since = clock.day_start(today - days + 1);
    let mut out = vec![[0u32; 3]; days as usize];
    let mut st = db.conn.prepare(&format!("SELECT at, kind FROM revlog WHERE undone = 0 AND at >= ?1 AND kind != 4{}", scope.cards("card_id")))?;
    for r in st.query_map([since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (at, kind) = r?;
        let i = clock.day_of(at) - (today - days + 1);
        if (0..days).contains(&i) {
            let slot = match kind {
                0 => 1,
                2 => 2,
                _ => 0,
            };
            out[i as usize][slot] += 1;
        }
    }
    Ok(out)
}

/// Revisões que vencem em cada um dos próximos `days` dias (atrasadas contam hoje).
pub fn forecast(db: &Db, scope: &Scope, clock: &Clock, days: i64) -> Result<Vec<u32>> {
    let today = clock.today();
    let mut out = vec![0u32; days as usize];
    let mut st = db.conn.prepare(&format!("SELECT due, state FROM cards WHERE queue = 0 AND state != 0{}", scope.decks()))?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (due, _) = r?;
        let i = (clock.day_of(due) - today).max(0);
        if i < days {
            out[i as usize] += 1;
        }
    }
    Ok(out)
}

/// Calibração: (R prevista média, acerto observado, nº de revisões) por faixa de 5%.
pub fn calibration(db: &Db, scope: &Scope, since: i64) -> Result<Vec<(f32, f32, u32)>> {
    let mut bins = [(0.0f64, 0u32, 0u32); 20];
    let mut st = db.conn.prepare(&format!(
        "SELECT retrievability, rating FROM revlog WHERE undone = 0 AND at >= ?1 AND kind = 1 AND retrievability IS NOT NULL{}",
        scope.cards("card_id")
    ))?;
    for r in st.query_map([since], |r| Ok((r.get::<_, f64>(0)?, r.get::<_, i64>(1)?)))? {
        let (p, rating) = r?;
        let i = ((p * 20.0).floor() as usize).min(19);
        bins[i].0 += p;
        bins[i].1 += (rating > 1) as u32;
        bins[i].2 += 1;
    }
    Ok(bins.iter().filter(|b| b.2 > 0).map(|b| ((b.0 / b.2 as f64) as f32, b.1 as f32 / b.2 as f32, b.2)).collect())
}

/// RMSE ponderado da calibração (mesma ideia do "RMSE (bins)" do FSRS).
pub fn calibration_rmse(c: &[(f32, f32, u32)]) -> Option<f32> {
    let total: u32 = c.iter().map(|b| b.2).sum();
    if total == 0 {
        return None;
    }
    Some((c.iter().map(|b| (b.0 - b.1).powi(2) * b.2 as f32).sum::<f32>() / total as f32).sqrt())
}

#[derive(Default, Clone, Copy, Debug)]
pub struct CardCounts {
    pub new: u32,
    pub learning: u32,
    pub young: u32,
    pub mature: u32,
    pub suspended: u32,
}

impl CardCounts {
    pub fn total(&self) -> u32 {
        self.new + self.learning + self.young + self.mature + self.suspended
    }
}

pub fn card_counts(db: &Db, scope: &Scope) -> Result<CardCounts> {
    let mut st = db.conn.prepare(&format!("SELECT state, queue, interval FROM cards WHERE 1{}", scope.decks()))?;
    let mut c = CardCounts::default();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?)))? {
        let (state, queue, ivl) = r?;
        if queue == -1 {
            c.suspended += 1;
        } else {
            match state {
                0 => c.new += 1,
                1 | 3 => c.learning += 1,
                _ if ivl >= 21.0 => c.mature += 1,
                _ => c.young += 1,
            }
        }
    }
    Ok(c)
}

/// Frequência dos botões: [aprendendo, jovens, maduros] × [Errei, Difícil, Bom, Fácil].
pub fn buttons(db: &Db, scope: &Scope, since: i64) -> Result<[[u32; 4]; 3]> {
    let mut out = [[0u32; 4]; 3];
    let mut st = db.conn.prepare(&format!("SELECT kind, scheduled_days, rating FROM revlog WHERE undone = 0 AND at >= ?1 AND kind != 4{}", scope.cards("card_id")))?;
    for r in st.query_map([since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?)))? {
        let (kind, sched, rating) = r?;
        let row = match kind {
            0 | 2 => 0,
            _ if sched >= 21.0 => 2,
            _ => 1,
        };
        let col = (rating.clamp(1, 4) - 1) as usize;
        out[row][col] += 1;
    }
    Ok(out)
}

/// Por hora do dia: (revisões, acertos).
pub fn hours(db: &Db, scope: &Scope, since: i64) -> Result<[(u32, u32); 24]> {
    use chrono::{Local, TimeZone, Timelike};
    let mut out = [(0u32, 0u32); 24];
    let mut st = db.conn.prepare(&format!("SELECT at, rating FROM revlog WHERE undone = 0 AND at >= ?1 AND kind != 4{}", scope.cards("card_id")))?;
    for r in st.query_map([since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (at, rating) = r?;
        if let Some(t) = Local.timestamp_millis_opt(at).single() {
            let h = t.hour() as usize;
            out[h].0 += 1;
            out[h].1 += (rating > 1) as u32;
        }
    }
    Ok(out)
}

/// Conhecimento retido: soma da R de todos os cartões já estudados, agora e há `back_days`.
pub fn retained(db: &Db, scope: &Scope, decay_of: &dyn Fn(i64) -> f32, back_days: i64) -> Result<(f32, f32)> {
    let now = srs::now_ms();
    let then = now - back_days * DAY_MS;
    let mut st = db.conn.prepare(&format!("SELECT deck_id, stability, difficulty, last_review FROM cards WHERE stability IS NOT NULL AND last_review IS NOT NULL AND queue != -1{}", scope.decks()))?;
    let (mut a, mut b) = (0.0f32, 0.0f32);
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?)))? {
        let (deck, s, d, last) = r?;
        let mem = fsrs::MemoryState { stability: s as f32, difficulty: d as f32 };
        let decay = decay_of(deck);
        a += fsrs::current_retrievability(mem, ((now - last) as f32 / DAY_MS as f32).max(0.0), decay);
        if last <= then {
            b += fsrs::current_retrievability(mem, ((then - last) as f32 / DAY_MS as f32).max(0.0), decay);
        }
    }
    Ok((a, b))
}

/// Distribuição de D, S e R: histogramas com `bins` faixas.
pub fn distributions(db: &Db, scope: &Scope, decay_of: &dyn Fn(i64) -> f32) -> Result<([u32; 10], [u32; 10], [u32; 10])> {
    let now = srs::now_ms();
    let (mut d_h, mut s_h, mut r_h) = ([0u32; 10], [0u32; 10], [0u32; 10]);
    let mut st = db.conn.prepare(&format!("SELECT deck_id, stability, difficulty, last_review FROM cards WHERE stability IS NOT NULL AND last_review IS NOT NULL{}", scope.decks()))?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?)))? {
        let (deck, s, d, last) = r?;
        d_h[(((d - 1.0) / 0.9).floor() as usize).min(9)] += 1;
        // Estabilidade em escala log: <1 d, 1–2, 2–4 … ≥ 256 d.
        let si = if s < 1.0 { 0 } else { ((s.log2().floor() as usize) + 1).min(9) };
        s_h[si] += 1;
        let rr = fsrs::current_retrievability(fsrs::MemoryState { stability: s as f32, difficulty: d as f32 }, ((now - last) as f32 / DAY_MS as f32).max(0.0), decay_of(deck));
        r_h[((rr * 10.0).floor() as usize).min(9)] += 1;
    }
    Ok((d_h, s_h, r_h))
}

/// Tempo médio por resposta nos últimos 30 dias (para estimar a sessão).
pub fn avg_answer_secs(db: &Db) -> f32 {
    let since = srs::now_ms() - 30 * DAY_MS;
    db.conn
        .query_row("SELECT AVG(duration_ms) FROM revlog WHERE undone = 0 AND at >= ?1", [since], |r| r.get::<_, Option<f64>>(0))
        .ok()
        .flatten()
        .map(|v| (v / 1000.0) as f32)
        .unwrap_or(10.0)
}
