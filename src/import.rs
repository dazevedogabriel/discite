//! Cartões e questões colados como JSON (o formato que se pede a um modelo
//! de IA externo). O formato completo, para copiar para a IA, está em
//! `assets/import-prompt.*.md`:
//!
//! ```json
//! {
//!   "cards": [
//!     {"type": "basic", "front": "…", "back": "…", "extra": "…", "tags": "…"},
//!     {"type": "cloze", "text": "A {{c1::…}} …", "extra": "…"}
//!   ],
//!   "questions": [
//!     {"stem": "…", "options": ["…", "…", "…", "…"], "answer": "B", "explanation": "…"}
//!   ]
//! }
//! ```

use crate::cards::NoteKind;
use crate::db::Note;
use crate::quiz::OPTIONS;
use crate::tr;
use serde_json::Value;

/// Questão lida (o banco e a página vêm do diálogo).
#[derive(Debug, PartialEq)]
pub struct Q {
    pub stem: String,
    pub options: [String; OPTIONS],
    pub answer: usize,
    pub explanation: String,
}

#[derive(Debug, Default)]
pub struct Batch {
    pub notes: Vec<Note>,
    pub questions: Vec<Q>,
    /// Itens pulados, com o motivo; ou o erro do JSON inteiro.
    pub errors: Vec<String>,
}

fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(t) => t.trim().to_owned(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Lê o texto colado. Aceita o JSON cercado de conversa ou de ```json.
pub fn parse(text: &str) -> Batch {
    let mut b = Batch::default();
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        if !text.trim().is_empty() {
            b.errors.push(tr!("import.no_json"));
        }
        return b;
    };
    let root: Value = match serde_json::from_str(&text[start..=end]) {
        Ok(v) => v,
        Err(e) => {
            b.errors.push(tr!("import.bad_json", error = e));
            return b;
        }
    };
    for (i, c) in root["cards"].as_array().into_iter().flatten().enumerate() {
        let n = i + 1;
        let kind = match s(c, "type").to_lowercase().as_str() {
            "cloze" | "lacuna" => NoteKind::Cloze,
            "reversed" | "basic_reversed" | "invertido" => NoteKind::BasicReversed,
            "typein" | "type_in" | "digitar" => NoteKind::TypeIn,
            "" if c.get("text").is_some() => NoteKind::Cloze,
            _ => NoteKind::Basic,
        };
        let mut note = Note::new(kind);
        note.fields = if kind == NoteKind::Cloze { vec![s(c, "text"), s(c, "extra")] } else { vec![s(c, "front"), s(c, "back"), s(c, "extra")] };
        note.tags = s(c, "tags");
        if note.fields[0].is_empty() || (kind != NoteKind::Cloze && note.fields[1].is_empty()) {
            b.errors.push(tr!("import.card_empty", n = n));
        } else if crate::cards::card_ords(kind, &note.fields).is_empty() {
            b.errors.push(tr!("import.card_no_gap", n = n));
        } else {
            b.notes.push(note);
        }
    }
    for (i, q) in root["questions"].as_array().into_iter().flatten().enumerate() {
        let n = i + 1;
        let opts: Vec<String> = q["options"].as_array().into_iter().flatten().map(|o| o.as_str().unwrap_or_default().trim().to_owned()).collect();
        let letter = s(q, "answer").to_uppercase();
        let answer = match letter.as_str() {
            "A" | "B" | "C" | "D" => Some((letter.as_bytes()[0] - b'A') as usize),
            _ => None,
        };
        let stem = s(q, "stem");
        if stem.is_empty() || opts.len() != OPTIONS || opts.iter().any(String::is_empty) {
            b.errors.push(tr!("import.question_shape", n = n));
        } else if let Some(answer) = answer {
            b.questions.push(Q { stem, options: std::array::from_fn(|k| opts[k].clone()), answer, explanation: s(q, "explanation") });
        } else {
            b.errors.push(tr!("import.question_answer", n = n));
        }
    }
    if b.notes.is_empty() && b.questions.is_empty() && b.errors.is_empty() {
        b.errors.push(tr!("import.nothing"));
    }
    b
}

/// As instruções para o modelo de IA, no idioma do app.
pub fn prompt() -> &'static str {
    if crate::i18n::current().starts_with("pt") { include_str!("../assets/import-prompt.pt-BR.md") } else { include_str!("../assets/import-prompt.en.md") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cards_and_questions_and_skips_bad_items() {
        let text = r#"Claro! Aqui está:
```json
{
  "cards": [
    {"type": "basic", "front": "Agente mais comum?", "back": "**Pneumococo**", "tags": ["meningite", "prova"]},
    {"type": "cloze", "text": "LCR com {{c1::glicose baixa}} sugere bactéria.", "extra": "razão < 0,4"},
    {"text": "sem tipo, vira lacuna: {{c1::ok}}"},
    {"type": "cloze", "text": "sem lacuna"},
    {"type": "basic", "front": "sem verso"}
  ],
  "questions": [
    {"stem": "Qual?", "options": ["a", "b", "c", "d"], "answer": "c", "explanation": "porque"},
    {"stem": "Três?", "options": ["a", "b", "c"], "answer": "A"},
    {"stem": "Letra?", "options": ["a", "b", "c", "d"], "answer": "E"}
  ]
}
```"#;
        let b = parse(text);
        assert_eq!(b.notes.len(), 3);
        assert_eq!(b.notes[0].tags, "meningite prova");
        assert_eq!(b.notes[1].kind, NoteKind::Cloze);
        assert_eq!(b.notes[1].fields[1], "razão < 0,4");
        assert_eq!(b.notes[2].kind, NoteKind::Cloze);
        assert_eq!(b.questions, vec![Q { stem: "Qual?".into(), options: ["a", "b", "c", "d"].map(String::from), answer: 2, explanation: "porque".into() }]);
        assert_eq!(b.errors.len(), 4, "{:?}", b.errors);
        assert_eq!(parse("{ nada").errors.len(), 1);
        assert!(parse("").errors.is_empty());
    }
}
