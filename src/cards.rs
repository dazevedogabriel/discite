//! Tipos de nota embutidos, geração de cartões a partir das notas e
//! renderização de frente/verso (cloze, digitar resposta, oclusão de imagem).

use serde::{Deserialize, Serialize};

/// Marcadores invisíveis que o texto rico entende como "lacuna" (fundo de
/// destaque). Ficam na área de uso privado do Unicode, então não colidem com
/// nada que o usuário digite.
pub const GAP_OPEN: char = '\u{E000}';
pub const GAP_CLOSE: char = '\u{E001}';

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum NoteKind {
    Basic,
    BasicReversed,
    Cloze,
    TypeIn,
    Occlusion,
}

impl NoteKind {
    pub const ALL: [NoteKind; 5] = [NoteKind::Basic, NoteKind::BasicReversed, NoteKind::Cloze, NoteKind::TypeIn, NoteKind::Occlusion];

    pub fn label(self) -> String {
        crate::tr!(match self {
            NoteKind::Basic => "kind.basic",
            NoteKind::BasicReversed => "kind.basic_reversed",
            NoteKind::Cloze => "kind.cloze",
            NoteKind::TypeIn => "kind.type_in",
            NoteKind::Occlusion => "kind.occlusion",
        })
    }

    /// Chaves de tradução dos campos (`tr!` dá o nome mostrado).
    pub fn field_names(self) -> &'static [&'static str] {
        match self {
            NoteKind::Basic | NoteKind::BasicReversed | NoteKind::TypeIn => &["field.front", "field.back", "field.extra"],
            NoteKind::Cloze => &["field.text", "field.extra"],
            NoteKind::Occlusion => &["field.image", "field.masks", "field.header", "field.extra"],
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            NoteKind::Basic => "basic",
            NoteKind::BasicReversed => "basic_reversed",
            NoteKind::Cloze => "cloze",
            NoteKind::TypeIn => "type_in",
            NoteKind::Occlusion => "occlusion",
        }
    }

    pub fn from_key(s: &str) -> NoteKind {
        match s {
            "basic_reversed" => NoteKind::BasicReversed,
            "cloze" => NoteKind::Cloze,
            "type_in" => NoteKind::TypeIn,
            "occlusion" => NoteKind::Occlusion,
            _ => NoteKind::Basic,
        }
    }

    /// Converte os campos ao trocar de tipo, mantendo o que der.
    pub fn convert_fields(from: NoteKind, to: NoteKind, fields: &[String]) -> Vec<String> {
        let get = |i: usize| fields.get(i).cloned().unwrap_or_default();
        let n = to.field_names().len();
        let mut out = vec![String::new(); n];
        match (from, to) {
            (a, b) if a == b => return (0..n).map(get).collect(),
            (NoteKind::Cloze, _) => {
                out[0] = strip_cloze(&get(0));
                if to != NoteKind::Occlusion {
                    out[n - 1] = get(1);
                }
            }
            (_, NoteKind::Cloze) => {
                let front = get(0);
                let back = get(1);
                out[0] = if back.trim().is_empty() { front } else { format!("{front}\n{back}") };
                out[1] = get(2);
            }
            (NoteKind::Occlusion, _) | (_, NoteKind::Occlusion) => {
                out[n - 1] = fields.last().cloned().unwrap_or_default();
            }
            _ => {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = get(i);
                }
            }
        }
        out
    }
}

/// Uma lacuna `{{c1::resposta::dica}}` dentro do texto (posições em bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct ClozeSpan {
    pub start: usize,
    pub end: usize,
    pub num: u32,
    pub answer: String,
    pub hint: Option<String>,
}

pub fn parse_clozes(text: &str) -> Vec<ClozeSpan> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find("{{c") {
        let start = i + off;
        let rest = &text[start + 3..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() || !rest[digits.len()..].starts_with("::") {
            i = start + 3;
            continue;
        }
        let body_start = start + 3 + digits.len() + 2;
        let Some(close) = text[body_start..].find("}}") else { break };
        let body = &text[body_start..body_start + close];
        let (answer, hint) = match body.split_once("::") {
            Some((a, h)) => (a.to_owned(), Some(h.to_owned())),
            None => (body.to_owned(), None),
        };
        let end = body_start + close + 2;
        out.push(ClozeSpan { start, end, num: digits.parse().unwrap_or(1).max(1), answer, hint });
        i = end;
    }
    out
}

pub fn cloze_numbers(text: &str) -> Vec<u32> {
    let mut v: Vec<u32> = parse_clozes(text).into_iter().map(|c| c.num).collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub fn strip_cloze(text: &str) -> String {
    render_cloze(text, 0, ClozeSide::Plain)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClozeSide {
    Front,
    Back,
    /// Só as respostas, sem marcação (busca, ordenação).
    Plain,
}

/// Frente: a lacuna `num` vira `[ … ]` (ou `[dica]`); as outras mostram a resposta.
/// Verso: a lacuna `num` aparece destacada.
pub fn render_cloze(text: &str, num: u32, side: ClozeSide) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for c in parse_clozes(text) {
        out.push_str(&text[last..c.start]);
        if c.num == num && side != ClozeSide::Plain {
            out.push(GAP_OPEN);
            match side {
                ClozeSide::Front => out.push_str(&c.hint.as_deref().map(|h| format!("[{h}]")).unwrap_or_else(|| "[ … ]".into())),
                _ => out.push_str(&c.answer),
            }
            out.push(GAP_CLOSE);
        } else {
            out.push_str(&c.answer);
        }
        last = c.end;
    }
    out.push_str(&text[last..]);
    out
}

/// Próximo número de lacuna livre (para "Nova lacuna").
pub fn next_cloze_number(text: &str) -> u32 {
    cloze_numbers(text).last().copied().unwrap_or(0) + 1
}

/// Retângulo de oclusão, em coordenadas normalizadas (0–1) da imagem.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn parse_masks(s: &str) -> Vec<Mask> {
    serde_json::from_str(s).unwrap_or_default()
}

pub fn masks_json(m: &[Mask]) -> String {
    serde_json::to_string(m).unwrap_or_else(|_| "[]".into())
}

/// Ordinais dos cartões que a nota gera.
pub fn card_ords(kind: NoteKind, fields: &[String]) -> Vec<u32> {
    let f = |i: usize| fields.get(i).map(String::as_str).unwrap_or("");
    match kind {
        NoteKind::Basic | NoteKind::TypeIn => vec![0],
        NoteKind::BasicReversed => vec![0, 1],
        NoteKind::Cloze => cloze_numbers(f(0)).into_iter().map(|n| n - 1).collect(),
        NoteKind::Occlusion => (0..parse_masks(f(1)).len() as u32).collect(),
    }
}

/// Primeiro campo em texto puro, para ordenar, buscar e detectar duplicatas.
pub fn sort_field(kind: NoteKind, fields: &[String]) -> String {
    let f0 = fields.first().map(String::as_str).unwrap_or("");
    let text = match kind {
        NoteKind::Cloze => strip_cloze(f0),
        NoteKind::Occlusion => fields.get(2).cloned().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| NoteKind::Occlusion.label()),
        _ => f0.to_owned(),
    };
    crate::rich::plain(&text)
}

pub fn checksum(text: &str) -> i64 {
    let norm: String = text.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in norm.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h >> 1) as i64
}

/// O que mostrar de um cartão. Os textos usam a marcação inline do Discite e
/// podem ter linhas `![](arquivo)` com imagens.
#[derive(Clone, Debug, Default)]
pub struct Face {
    pub question: String,
    /// Resposta principal (vazia no cloze: o verso é a própria frase).
    pub answer: String,
    pub extra: String,
    /// Cloze: frase completa com a lacuna destacada.
    pub cloze_back: Option<String>,
    /// Digitar resposta: o que comparar.
    pub type_answer: Option<String>,
    /// Oclusão: imagem, máscaras e qual está em jogo.
    pub occlusion: Option<(String, Vec<Mask>, usize)>,
}

pub fn render(kind: NoteKind, fields: &[String], ord: u32) -> Face {
    let f = |i: usize| fields.get(i).cloned().unwrap_or_default();
    match kind {
        NoteKind::Basic => Face { question: f(0), answer: f(1), extra: f(2), ..Default::default() },
        NoteKind::BasicReversed => {
            if ord == 1 {
                Face { question: f(1), answer: f(0), extra: f(2), ..Default::default() }
            } else {
                Face { question: f(0), answer: f(1), extra: f(2), ..Default::default() }
            }
        }
        NoteKind::TypeIn => Face { question: f(0), answer: f(1), extra: f(2), type_answer: Some(crate::rich::plain(&f(1))), ..Default::default() },
        NoteKind::Cloze => {
            let num = ord + 1;
            Face {
                question: render_cloze(&f(0), num, ClozeSide::Front),
                cloze_back: Some(render_cloze(&f(0), num, ClozeSide::Back)),
                extra: f(1),
                ..Default::default()
            }
        }
        NoteKind::Occlusion => Face {
            question: f(2),
            extra: f(3),
            occlusion: Some((f(0), parse_masks(&f(1)), ord as usize)),
            ..Default::default()
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Diff {
    Same,
    /// Digitado a mais / errado.
    Wrong,
    /// Faltou digitar.
    Missing,
}

/// Comparação caractere a caractere (LCS) entre o digitado e o esperado.
pub fn diff(typed: &str, expected: &str) -> (Vec<(char, Diff)>, Vec<(char, Diff)>) {
    let a: Vec<char> = typed.trim().chars().collect();
    let b: Vec<char> = expected.trim().chars().collect();
    let eq = |x: char, y: char| x.to_lowercase().eq(y.to_lowercase());
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if eq(a[i], b[j]) { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ta = Vec::new();
    let mut tb = Vec::new();
    while i < n && j < m {
        if eq(a[i], b[j]) {
            ta.push((a[i], Diff::Same));
            tb.push((b[j], Diff::Same));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ta.push((a[i], Diff::Wrong));
            i += 1;
        } else {
            tb.push((b[j], Diff::Missing));
            j += 1;
        }
    }
    ta.extend(a[i..].iter().map(|c| (*c, Diff::Wrong)));
    tb.extend(b[j..].iter().map(|c| (*c, Diff::Missing)));
    (ta, tb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloze_parsing_and_rendering() {
        let t = "Na {{c1::metáfase}}, os cromossomos se alinham na {{c2::placa equatorial}}, presos pelos {{c3::cinetócoros::estrutura}}.";
        assert_eq!(cloze_numbers(t), vec![1, 2, 3]);
        assert_eq!(card_ords(NoteKind::Cloze, &[t.into(), String::new()]), vec![0, 1, 2]);
        let front = render_cloze(t, 3, ClozeSide::Front);
        assert!(front.contains("metáfase") && front.contains("[estrutura]") && !front.contains("cinetócoros"));
        let back = render_cloze(t, 1, ClozeSide::Back);
        assert!(back.contains(&format!("{GAP_OPEN}metáfase{GAP_CLOSE}")));
        assert_eq!(strip_cloze(t), "Na metáfase, os cromossomos se alinham na placa equatorial, presos pelos cinetócoros.");
        assert_eq!(next_cloze_number(t), 4);
    }

    #[test]
    fn diff_marks_errors() {
        let (typed, exp) = diff("angiotensna II", "angiotensina II");
        assert!(typed.iter().all(|(_, d)| *d == Diff::Same));
        assert_eq!(exp.iter().filter(|(_, d)| *d == Diff::Missing).count(), 1);
    }

    #[test]
    fn checksum_ignores_case_and_spacing() {
        assert_eq!(checksum("Olá  Mundo"), checksum("olá mundo "));
    }
}
