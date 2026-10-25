//! Linguagem de busca do navegador de cartões, traduzida para SQL.
//!
//! `deck:Farmacologia is:due prop:r<0.8 lapses>1 tag:prova added:7 "texto exato"`
//!
//! Termos: `deck:` / `baralho:`, `subject:` / `disciplina:`, `tag:`, `is:`
//! (new, learn, review, due, suspended, buried, flagged/marcado, leech),
//! `prop:` (r, d, s, ivl, reps, lapses, due — dias a partir de hoje),
//! `lapses>N`, `reps>N`, `added:N`, `rated:N[:nota]`, `forgot:N`, `note:`
//! (tipo), palavras soltas ou entre aspas (texto completo) e `-` para negar.

use crate::db::{self, Deck, Subject};
use crate::srs::Clock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Cmp {
    fn sql(self) -> &'static str {
        match self {
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
            Cmp::Eq => "=",
        }
    }
    pub fn test(self, a: f32, b: f32) -> bool {
        match self {
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Eq => (a - b).abs() < 1e-6,
        }
    }
}

#[derive(Debug, Default)]
pub struct Query {
    /// Cláusula WHERE sobre `cards c JOIN notes n`.
    pub sql: String,
    /// `prop:r…`: filtrado depois, em Rust (R depende do preset).
    pub r_filter: Vec<(Cmp, f32, bool)>,
    pub errors: Vec<String>,
}

fn split_terms(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_cmp(s: &str) -> Option<(Cmp, f32)> {
    let (cmp, rest) = if let Some(r) = s.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = s.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (Cmp::Gt, r)
    } else if let Some(r) = s.strip_prefix('=') {
        (Cmp::Eq, r)
    } else if let Some(r) = s.strip_prefix(':') {
        (Cmp::Eq, r)
    } else {
        return None;
    };
    let v: f32 = rest.trim_end_matches('%').replace(',', ".").parse().ok()?;
    let v = if rest.ends_with('%') { v / 100.0 } else { v };
    Some((cmp, v))
}

fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

pub fn parse(q: &str, decks: &[Deck], subjects: &[Subject], clock: &Clock, now: i64) -> Query {
    let mut parts: Vec<String> = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut out = Query::default();
    let day_end = clock.today_end();
    let day_start = clock.day_start(clock.today());
    for raw in split_terms(q) {
        let (neg, term) = match raw.strip_prefix('-') {
            Some(t) if !t.is_empty() => (true, t.to_owned()),
            _ => (false, raw.clone()),
        };
        let lower = term.to_lowercase();
        let (key, val) = match lower.split_once(':') {
            Some((k, _)) if !k.starts_with('"') => (k.to_owned(), term[k.len() + 1..].to_owned()),
            _ => (String::new(), term.clone()),
        };
        let clause: Option<String> = match key.as_str() {
            "deck" | "baralho" => {
                let name = val.trim_matches('"').to_lowercase();
                let wild = name.ends_with('*');
                let name = name.trim_end_matches('*');
                let roots: Vec<i64> = decks
                    .iter()
                    .filter(|d| {
                        let n = d.name.to_lowercase();
                        let full = deck_path(decks, d.id).to_lowercase();
                        if wild { n.starts_with(name) || full.starts_with(name) } else { n == name || full == name }
                    })
                    .map(|d| d.id)
                    .collect();
                let mut ids: Vec<i64> = roots.iter().flat_map(|r| db::subtree(decks, *r)).collect();
                ids.sort_unstable();
                ids.dedup();
                if ids.is_empty() {
                    out.errors.push(format!("Baralho não encontrado: {val}"));
                    Some("0".into())
                } else {
                    Some(format!("c.deck_id IN ({})", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")))
                }
            }
            "subject" | "disciplina" => {
                let name = val.trim_matches('"').to_lowercase();
                let sids: Vec<i64> = subjects.iter().filter(|s| s.name.to_lowercase().starts_with(&name)).map(|s| s.id).collect();
                let ids: Vec<String> = decks.iter().filter(|d| sids.contains(&d.subject_id)).map(|d| d.id.to_string()).collect();
                if ids.is_empty() {
                    out.errors.push(format!("Disciplina não encontrada: {val}"));
                    Some("0".into())
                } else {
                    Some(format!("c.deck_id IN ({})", ids.join(",")))
                }
            }
            "tag" => {
                let t = esc(&val.trim_start_matches('#').to_lowercase());
                if let Some(p) = t.strip_suffix('*') {
                    Some(format!("(' ' || lower(n.tags) || ' ') LIKE '% {p}%'"))
                } else {
                    Some(format!("(' ' || lower(n.tags) || ' ') LIKE '% {t} %'"))
                }
            }
            "is" => match val.to_lowercase().as_str() {
                "new" | "novo" | "novos" => Some("c.state = 0".into()),
                "learn" | "aprendendo" => Some("c.state IN (1, 3)".into()),
                "review" | "revisão" | "revisao" => Some("c.state = 2".into()),
                "due" | "vencido" | "vencidos" => Some(format!("c.queue = 0 AND ((c.state = 2 AND c.due < {day_end}) OR (c.state IN (1, 3) AND c.due <= {now}))")),
                "suspended" | "suspenso" | "suspensos" => Some("c.queue = -1".into()),
                "buried" | "enterrado" => Some("c.queue IN (-2, -3)".into()),
                "flagged" | "marked" | "marcado" | "marcados" => Some("c.flagged = 1".into()),
                "leech" | "sanguessuga" => Some("c.leech = 1".into()),
                other => {
                    out.errors.push(format!("is:{other} desconhecido"));
                    None
                }
            },
            "prop" => {
                let lv = val.to_lowercase();
                let field: String = lv.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                match parse_cmp(&lv[field.len()..]) {
                    Some((cmp, v)) => match field.as_str() {
                        "r" => {
                            out.r_filter.push((cmp, v, neg));
                            None
                        }
                        "d" => Some(format!("c.difficulty {} {v}", cmp.sql())),
                        "s" => Some(format!("c.stability {} {v}", cmp.sql())),
                        "ivl" => Some(format!("c.interval {} {v}", cmp.sql())),
                        "reps" => Some(format!("c.reps {} {v}", cmp.sql())),
                        "lapses" => Some(format!("c.lapses {} {v}", cmp.sql())),
                        "due" => {
                            let t = clock.day_start(clock.today() + v as i64 + 1);
                            Some(format!("c.state = 2 AND c.due {} {t}", if matches!(cmp, Cmp::Gt | Cmp::Ge) { ">=" } else { "<" }))
                        }
                        _ => {
                            out.errors.push(format!("prop:{field} desconhecido"));
                            None
                        }
                    },
                    None => {
                        out.errors.push(format!("Comparação inválida em {raw}"));
                        None
                    }
                }
            }
            "added" | "adicionado" => {
                let days: i64 = val.parse().unwrap_or(1).max(1);
                Some(format!("c.created_at >= {}", clock.day_start(clock.today() - days + 1)))
            }
            "rated" | "respondido" | "forgot" | "esquecido" => {
                let mut it = val.split(':');
                let days: i64 = it.next().and_then(|d| d.parse().ok()).unwrap_or(1).max(1);
                let rating: Option<u32> = if key == "forgot" || key == "esquecido" { Some(1) } else { it.next().and_then(|r| r.parse().ok()) };
                let since = clock.day_start(clock.today() - days + 1);
                let r = rating.map(|r| format!(" AND rating = {r}")).unwrap_or_default();
                Some(format!("c.id IN (SELECT card_id FROM revlog WHERE undone = 0 AND at >= {since}{r})"))
            }
            "nid" => Some(format!("c.note_id = {}", val.parse::<i64>().unwrap_or(0))),
            "cid" => {
                let ids: Vec<String> = val.split(',').filter_map(|v| v.parse::<i64>().ok()).map(|v| v.to_string()).collect();
                Some(if ids.is_empty() { "0".into() } else { format!("c.id IN ({})", ids.join(",")) })
            }
            "page" | "pagina" => Some(format!("n.source_page = {}", val.parse::<i64>().unwrap_or(0))),
            "note" | "tipo" => {
                let v = val.to_lowercase();
                let kinds: Vec<&str> = crate::cards::NoteKind::ALL
                    .iter()
                    .filter(|k| k.label().to_lowercase().starts_with(&v) || k.key().starts_with(&v))
                    .map(|k| k.key())
                    .collect();
                Some(if kinds.is_empty() { "0".into() } else { format!("n.kind IN ({})", kinds.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(",")) })
            }
            _ => {
                if let Some(rest) = lower.strip_prefix("lapses").or_else(|| lower.strip_prefix("lapsos")) {
                    parse_cmp(rest).map(|(c, v)| format!("c.lapses {} {v}", c.sql()))
                } else if let Some(rest) = lower.strip_prefix("reps") {
                    parse_cmp(rest).map(|(c, v)| format!("c.reps {} {v}", c.sql()))
                } else {
                    if neg {
                        let q = db::fts_query(&term.replace('"', ""));
                        if !q.is_empty() {
                            parts.push(format!("n.id NOT IN (SELECT note_id FROM fts_notes WHERE fts_notes MATCH '{}')", esc(&q)));
                        }
                    } else {
                        text.push(term.replace('"', ""));
                    }
                    continue;
                }
            }
        };
        if let Some(c) = clause {
            parts.push(if neg { format!("NOT ({c})") } else { format!("({c})") });
        }
    }
    if !text.is_empty() {
        let q = db::fts_query(&text.join(" "));
        if !q.is_empty() {
            parts.push(format!("n.id IN (SELECT note_id FROM fts_notes WHERE fts_notes MATCH '{}')", esc(&q)));
        }
    }
    let _ = day_start;
    out.sql = if parts.is_empty() { "1".into() } else { parts.join(" AND ") };
    out
}

/// "Farmacologia › Cardiovascular › Anti-hipertensivos"
pub fn deck_path(decks: &[Deck], id: i64) -> String {
    let mut names = Vec::new();
    let mut cur = Some(id);
    let mut guard = 0;
    while let Some(c) = cur {
        let Some(d) = decks.iter().find(|d| d.id == c) else { break };
        names.push(d.name.clone());
        cur = d.parent_id;
        guard += 1;
        if guard > 32 {
            break;
        }
    }
    names.reverse();
    names.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sql() {
        let decks = vec![
            Deck { id: 1, subject_id: 9, parent_id: None, preset_id: 1, name: "Farmacologia".into(), position: 0, collapsed: false },
            Deck { id: 2, subject_id: 9, parent_id: Some(1), preset_id: 1, name: "Cardio".into(), position: 1, collapsed: false },
        ];
        let clock = Clock { rollover_hour: 4 };
        let q = parse("deck:Farmacologia is:due prop:r<0.8 lapses>1 -tag:fácil", &decks, &[], &clock, 0);
        assert!(q.sql.contains("c.deck_id IN (1,2)"), "{}", q.sql);
        assert!(q.sql.contains("c.lapses > 1"));
        assert!(q.sql.contains("NOT ("));
        assert_eq!(q.r_filter, vec![(Cmp::Lt, 0.8, false)]);
        assert!(q.errors.is_empty());
    }
}
