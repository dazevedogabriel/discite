//! O que o assistente pede para mudar (criar, editar e apagar cartões e
//! questões). O laço só propõe: quem grava é a interface, depois da
//! permissão, e cada ação devolve como se desfazer.

use crate::cards::NoteKind;
use crate::db::{Card, Db, Note};
use crate::quiz::Question;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewCard {
    pub cloze: bool,
    /// Básico: frente. Lacuna: o texto com `{{c1::…}}`.
    pub front: String,
    pub back: String,
    pub page: Option<i64>,
    pub block: Option<i64>,
    /// Veio de uma imagem anexada: não dá para conferir com as notas.
    pub from_image: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewQuestion {
    pub stem: String,
    pub options: [String; 4],
    pub answer: usize,
    pub explanation: String,
    pub page: Option<i64>,
    pub block: Option<i64>,
    pub from_image: bool,
}

impl NewQuestion {
    pub fn of(q: &Question) -> NewQuestion {
        NewQuestion { stem: q.stem.clone(), options: q.options.clone(), answer: q.answer, explanation: q.explanation.clone(), page: q.source_page, block: q.source_block, from_image: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    CreateCards { deck_id: i64, deck: String, cards: Vec<NewCard> },
    /// `before`/`after`: frente e verso (lacuna: texto e extra).
    EditCard { note_id: i64, cloze: bool, before: [String; 2], after: [String; 2] },
    DeleteCards { note_ids: Vec<i64>, previews: Vec<String> },
    CreateQuestions { bank_id: i64, bank: String, questions: Vec<NewQuestion> },
    EditQuestion { id: i64, before: NewQuestion, after: NewQuestion },
    DeleteQuestions { ids: Vec<i64>, previews: Vec<String> },
}

/// Como voltar atrás depois de aplicar.
pub enum Undo {
    DeleteNotes(Vec<i64>),
    RestoreNote(Note),
    RestoreNotes(Vec<Note>, Vec<Card>),
    DeleteQuestions(Vec<i64>),
    RestoreQuestion(Question),
    RestoreQuestions(Vec<Question>),
}

impl Action {
    /// Apagar tem regra de permissão própria.
    pub fn deletes(&self) -> bool {
        matches!(self, Action::DeleteCards { .. } | Action::DeleteQuestions { .. })
    }

    /// Grava. Devolve o resumo para o modelo (em inglês) e como desfazer.
    pub fn apply(&self, db: &Db) -> Result<(String, Undo)> {
        match self {
            Action::CreateCards { deck_id, deck, cards } => {
                let mut ids = vec![];
                for c in cards {
                    let mut n = Note::new(if c.cloze { NoteKind::Cloze } else { NoteKind::Basic });
                    n.fields = if c.cloze { vec![c.front.clone(), c.back.clone()] } else { vec![c.front.clone(), c.back.clone(), String::new()] };
                    n.source_page = c.page;
                    n.source_block = c.block;
                    db.add_note(&mut n, *deck_id)?;
                    ids.push(n.id);
                }
                Ok((format!("Created {} notes in deck \"{deck}\" (note_ids {:?}).", ids.len(), ids), Undo::DeleteNotes(ids)))
            }
            Action::EditCard { note_id, after, .. } => {
                let old = db.note(*note_id)?.ok_or_else(|| anyhow!("note {note_id} not found"))?;
                let deck = db.note_cards(*note_id)?.first().map(|c| c.deck_id).ok_or_else(|| anyhow!("note {note_id} has no cards"))?;
                let mut n = old.clone();
                n.fields[0] = after[0].clone();
                if n.fields.len() > 1 {
                    n.fields[1] = after[1].clone();
                }
                db.update_note(&mut n, deck)?;
                Ok((format!("Updated note {note_id}."), Undo::RestoreNote(old)))
            }
            Action::DeleteCards { note_ids, .. } => {
                let notes: Vec<Note> = note_ids.iter().filter_map(|id| db.note(*id).ok().flatten()).collect();
                let mut cards = vec![];
                for n in &notes {
                    cards.extend(db.note_cards(n.id)?);
                }
                db.delete_notes(note_ids)?;
                Ok((format!("Deleted {} notes.", notes.len()), Undo::RestoreNotes(notes, cards)))
            }
            Action::CreateQuestions { bank_id, bank, questions } => {
                let mut ids = vec![];
                for nq in questions {
                    let mut q = Question::new(*bank_id);
                    q.stem = nq.stem.clone();
                    q.options = nq.options.clone();
                    q.answer = nq.answer.min(3);
                    q.explanation = nq.explanation.clone();
                    q.source_page = nq.page;
                    q.source_block = nq.block;
                    ids.push(db.save_question(&mut q)?);
                }
                Ok((format!("Created {} questions in bank \"{bank}\" (question_ids {:?}).", ids.len(), ids), Undo::DeleteQuestions(ids)))
            }
            Action::EditQuestion { id, after, .. } => {
                let old = db.question(*id)?.ok_or_else(|| anyhow!("question {id} not found"))?;
                let mut q = old.clone();
                q.stem = after.stem.clone();
                q.options = after.options.clone();
                q.answer = after.answer.min(3);
                q.explanation = after.explanation.clone();
                db.save_question(&mut q)?;
                Ok((format!("Updated question {id}."), Undo::RestoreQuestion(old)))
            }
            Action::DeleteQuestions { ids, .. } => {
                let qs: Vec<Question> = ids.iter().filter_map(|id| db.question(*id).ok().flatten()).collect();
                db.delete_questions(ids)?;
                Ok((format!("Deleted {} questions.", qs.len()), Undo::RestoreQuestions(qs)))
            }
        }
    }
}

impl Undo {
    pub fn revert(self, db: &Db) -> Result<()> {
        match self {
            Undo::DeleteNotes(ids) => db.delete_notes(&ids),
            Undo::RestoreNote(mut n) => {
                let deck = db.note_cards(n.id)?.first().map(|c| c.deck_id).unwrap_or(1);
                db.update_note(&mut n, deck).map(|_| ())
            }
            Undo::RestoreNotes(notes, cards) => db.restore_notes(&notes, &cards),
            Undo::DeleteQuestions(ids) => db.delete_questions(&ids),
            Undo::RestoreQuestion(mut q) => db.save_question(&mut q).map(|_| ()),
            Undo::RestoreQuestions(qs) => qs.iter().try_for_each(|q| db.restore_question(q)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Db, i64, i64) {
        let dir = std::env::temp_dir().join(format!("discite-actions-{}", crate::db::new_id()));
        let db = Db::open(&dir).unwrap();
        let s = db.add_subject("Bio", 0).unwrap();
        let deck = db.list_decks().unwrap().into_iter().find(|d| d.subject_id == s).unwrap().id;
        let bank = db.first_bank(s, "Bio").unwrap();
        (db, deck, bank)
    }

    fn card(front: &str, back: &str, cloze: bool) -> NewCard {
        NewCard { cloze, front: front.into(), back: back.into(), page: None, block: None, from_image: false }
    }

    #[test]
    fn cards_create_edit_delete_and_undo() {
        let (db, deck, _) = setup();
        let create = Action::CreateCards { deck_id: deck, deck: "Bio".into(), cards: vec![card("A?", "B", false), card("A {{c1::mitose}} divide", "", true)] };
        let (_, undo) = create.apply(&db).unwrap();
        let Undo::DeleteNotes(ids) = &undo else { panic!() };
        assert_eq!(ids.len(), 2);
        let (basic, cloze) = (ids[0], ids[1]);
        assert_eq!(db.note(cloze).unwrap().unwrap().kind, NoteKind::Cloze);

        let edit = Action::EditCard { note_id: basic, cloze: false, before: ["A?".into(), "B".into()], after: ["A?".into(), "C".into()] };
        let (_, u) = edit.apply(&db).unwrap();
        assert_eq!(db.note(basic).unwrap().unwrap().fields[1], "C");
        u.revert(&db).unwrap();
        assert_eq!(db.note(basic).unwrap().unwrap().fields[1], "B");

        let del = Action::DeleteCards { note_ids: vec![basic], previews: vec![] };
        let (_, u) = del.apply(&db).unwrap();
        assert!(db.note(basic).unwrap().is_none());
        u.revert(&db).unwrap();
        assert_eq!(db.note_cards(basic).unwrap().len(), 1);

        undo.revert(&db).unwrap();
        assert!(db.note(basic).unwrap().is_none() && db.note(cloze).unwrap().is_none());
    }

    #[test]
    fn questions_create_edit_delete_and_undo() {
        let (db, _, bank) = setup();
        let nq = NewQuestion { stem: "Qual?".into(), options: ["a".into(), "b".into(), "c".into(), "d".into()], answer: 2, explanation: "".into(), page: None, block: None, from_image: false };
        let (_, undo) = Action::CreateQuestions { bank_id: bank, bank: "Bio".into(), questions: vec![nq.clone()] }.apply(&db).unwrap();
        let Undo::DeleteQuestions(ids) = &undo else { panic!() };
        let id = ids[0];
        let mut after = nq.clone();
        after.answer = 1;
        let (_, u) = Action::EditQuestion { id, before: nq.clone(), after }.apply(&db).unwrap();
        assert_eq!(db.question(id).unwrap().unwrap().answer, 1);
        u.revert(&db).unwrap();
        assert_eq!(db.question(id).unwrap().unwrap().answer, 2);
        let (_, u) = Action::DeleteQuestions { ids: vec![id], previews: vec![] }.apply(&db).unwrap();
        assert!(db.question(id).unwrap().is_none());
        u.revert(&db).unwrap();
        assert_eq!(db.question(id).unwrap().unwrap().stem, "Qual?");
        undo.revert(&db).unwrap();
        assert!(db.question(id).unwrap().is_none());
    }
}
