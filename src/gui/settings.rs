//! Configurações (aparência, estudo, dados e backup) e o conteúdo de exemplo.

use super::theme::*;
use super::widgets::*;
use super::App;
use crate::cards::NoteKind;
use crate::db::{Block, BlockKind, Db, Note, Rev};
use crate::srs::{self, Clock, DAY_MS, PresetConfig, Sched, State};
use eframe::egui::{self, Sense, Ui, vec2};
use rand::{Rng, SeedableRng};

pub fn ui(app: &mut App, ui: &mut Ui) {
    let p = pal();
    super::header(ui, "Configurações", &format!("Discite {} · dados em {}", super::VERSION, app.db.root.display()), |_| {});
    let area = ui.available_rect_before_wrap();
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(28.0, 20.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut cu, |ui| {
        ui.set_max_width(760.0);
        ui.spacing_mut().item_spacing.y = 16.0;
        section(ui, "Aparência", |ui| {
            row(ui, "Tema", |ui| {
                let mut sel = if app.cfg.dark { 0 } else { 1 };
                if segmented(ui, &["Escuro · grafite", "Claro · papel"], &mut sel, 28.0, false) {
                    app.cfg.dark = sel == 0;
                    app.save_settings();
                    apply(ui.ctx(), app.cfg.dark);
                }
            });
            row(ui, "Largura da coluna de leitura", |ui| {
                if ui.add(egui::Slider::new(&mut app.cfg.reading_width, 520.0..=900.0).suffix(" px").step_by(20.0)).changed() {
                    app.save_settings();
                }
            });
            row(ui, "Barra lateral recolhida", |ui| {
                if toggle(ui, &mut app.cfg.sidebar_collapsed, "Barra lateral recolhida").clicked() {
                    app.save_settings();
                }
            });
        });
        section(ui, "Estudo", |ui| {
            row(ui, "O dia de estudo vira às", |ui| {
                let mut h = app.cfg.rollover_hour;
                if ui.add(egui::DragValue::new(&mut h).range(0..=12).suffix(":00")).changed() {
                    app.cfg.rollover_hour = h;
                    app.save_settings();
                    app.changed();
                }
            });
            row(ui, "Teto do cronômetro por cartão", |ui| {
                let mut s = app.cfg.answer_cap_secs;
                if ui.add(egui::DragValue::new(&mut s).range(10..=600).suffix(" s")).changed() {
                    app.cfg.answer_cap_secs = s;
                    app.save_settings();
                }
            });
            row(ui, "Presets de agendamento", |ui| {
                if button(ui, "Abrir opções do preset", Btn::Secondary, 32.0).clicked() {
                    let id = app.presets.first().map(|p| p.id).unwrap_or(1);
                    app.go(super::Screen::Preset(id));
                }
            });
        });
        section(ui, "Dados e backup", |ui| {
            row(ui, "Backup automático (a cada 6 h de uso e ao fechar)", |ui| {
                if toggle(ui, &mut app.cfg.auto_backup, "Backup automático").clicked() {
                    app.save_settings();
                }
            });
            row(ui, &format!("Último backup: {}", if app.cfg.last_backup > 0 { ago(app.cfg.last_backup) } else { "nunca".into() }), |ui| {
                if button(ui, "Fazer backup agora", Btn::Secondary, 32.0).clicked() {
                    app.maybe_backup(true);
                    app.toast("Backup feito");
                }
            });
            row(ui, "Restaurar um backup (aplicado ao reabrir o Discite)", |ui| {
                if button(ui, "Restaurar backup…", Btn::Secondary, 32.0).clicked() {
                    let dir = app.db.root.join("backups");
                    if let Some(path) = rfd::FileDialog::new().set_directory(&dir).add_filter("Banco do Discite", &["db"]).pick_file() {
                        match std::fs::write(app.db.root.join("restore.pending"), path.to_string_lossy().as_bytes()) {
                            Ok(()) => app.toast("Feche e abra o Discite para concluir a restauração (o banco atual vai para backups)."),
                            Err(e) => app.error(e),
                        }
                    }
                }
            });
            row(ui, "Pasta de dados", |ui| {
                if button(ui, "Abrir pasta", Btn::Secondary, 32.0).clicked() {
                    open_folder(&app.db.root);
                }
            });
            row(ui, "Imagens que nada mais usa", |ui| {
                if button(ui, "Limpar mídia órfã", Btn::Secondary, 32.0).clicked() {
                    match app.db.orphan_media() {
                        Ok(list) => {
                            for f in &list {
                                let _ = std::fs::remove_file(app.db.media_dir().join(f));
                            }
                            app.toast(format!("{} arquivos removidos", list.len()));
                        }
                        Err(e) => app.error(e),
                    }
                }
            });
            row(ui, "Integridade do banco", |ui| {
                if button(ui, "Verificar", Btn::Secondary, 32.0).clicked() {
                    let r: String = app.db.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap_or_else(|e| e.to_string());
                    if r == "ok" {
                        app.toast("Banco íntegro");
                    } else {
                        app.error(format!("Problema no banco: {r}"));
                    }
                }
            });
            if app.subjects.is_empty() {
                row(ui, "Conteúdo de exemplo", |ui| {
                    if button(ui, "Carregar exemplo", Btn::Secondary, 32.0).clicked() {
                        match seed_demo(&app.db) {
                            Ok(()) => {
                                app.reload();
                                app.toast("Exemplo carregado");
                            }
                            Err(e) => app.error(e),
                        }
                    }
                });
            }
        });
        section(ui, "Sobre", |ui| {
            para(
                ui,
                "O agendamento usa o FSRS-6 (Free Spaced Repetition Scheduler), o mesmo modelo do Anki, com passos de aprendizado, fuzz e balanceamento de carga. Cada resposta fica registrada (nota, tempo, estado antes e depois), e o otimizador ajusta os parâmetros ao seu histórico.",
                serif(13.0, W::Regular),
                p.text2,
            );
            para(ui, "Fontes: Literata e JetBrains Mono (SIL Open Font License).", serif(12.0, W::Italic), p.muted);
        });
    });
}

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    card_frame().inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 10.0;
        label(ui, title, serif(16.0, W::Semi), p.text);
        add(ui);
    });
}

fn row(ui: &mut Ui, label_s: &str, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    ui.horizontal(|ui| {
        ui.set_min_height(34.0);
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w * 0.55, 20.0), Sense::hover());
        text(ui.painter(), r.left_center(), egui::Align2::LEFT_CENTER, label_s, serif(13.5, W::Regular), p.text2, r.width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

fn open_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

// ───────────────────────────── exemplo ─────────────────────────────

fn basic(front: &str, back: &str, extra: &str, tags: &str) -> Note {
    let mut n = Note::new(NoteKind::Basic);
    n.fields = vec![front.into(), back.into(), extra.into()];
    n.tags = tags.into();
    n
}

fn cloze(text: &str, extra: &str, tags: &str) -> Note {
    let mut n = Note::new(NoteKind::Cloze);
    n.fields = vec![text.into(), extra.into()];
    n.tags = tags.into();
    n
}

/// Quatro disciplinas do design com páginas, baralhos, notas e um histórico de
/// revisões simulado com o próprio agendador (para as estatísticas e o
/// otimizador terem o que mostrar).
pub fn seed_demo(db: &Db) -> anyhow::Result<()> {
    let anat = db.add_subject("Anatomia", 0)?;
    let farm = db.add_subject("Farmacologia", 1)?;
    let bio = db.add_subject("Biologia Celular", 2)?;
    let dir = db.add_subject("Direito Constitucional", 3)?;
    let decks = db.list_decks()?;
    let root = |s: i64| decks.iter().find(|d| d.subject_id == s).map(|d| d.id).unwrap();
    let med = db.add_preset("Medicina", &PresetConfig::default(), &[])?;
    for s in [anat, farm, bio] {
        if let Some(mut d) = db.list_decks()?.into_iter().find(|d| d.id == root(s)) {
            d.preset_id = med;
            db.update_deck(&d)?;
        }
    }

    // Baralhos de Farmacologia (como no design).
    let f = root(farm);
    let cardio = db.add_deck(farm, Some(f), "Cardiovascular")?;
    let antihip = db.add_deck(farm, Some(cardio), "Anti-hipertensivos")?;
    let antiarr = db.add_deck(farm, Some(cardio), "Antiarrítmicos")?;
    let sna = db.add_deck(farm, Some(f), "Sistema nervoso autônomo")?;
    let colin = db.add_deck(farm, Some(sna), "Colinérgicos")?;
    let adren = db.add_deck(farm, Some(sna), "Adrenérgicos")?;
    let antimic = db.add_deck(farm, Some(f), "Antimicrobianos")?;
    let _pk = db.add_deck(farm, Some(f), "Farmacocinética")?;
    let b = root(bio);
    let divisao = db.add_deck(bio, Some(b), "Divisão celular")?;
    let a = root(anat);
    let membros = db.add_deck(anat, Some(a), "Membro superior")?;
    let d = root(dir);

    // Páginas de Biologia Celular.
    let pg = |s: i64, parent: Option<i64>, t: &str| db.add_page(s, parent, t);
    let membrana = pg(bio, None, "Membrana plasmática")?;
    let organelas = pg(bio, None, "Organelas")?;
    let mito = pg(bio, Some(organelas), "Mitocôndria")?;
    let _ret = pg(bio, Some(organelas), "Retículo endoplasmático")?;
    let ciclo = pg(bio, None, "Ciclo celular e mitose")?;
    let meiose = pg(bio, Some(ciclo), "Meiose")?;
    let check = pg(bio, Some(ciclo), "Checkpoints e câncer")?;
    let _apop = pg(bio, None, "Apoptose")?;
    let _ex = pg(bio, None, "Exercícios — divisão")?;
    let mut blocks = vec![
        Block::new(BlockKind::Paragraph, "O ciclo celular é a sequência de eventos pela qual uma célula duplica seu conteúdo e se divide em duas. Ele se divide em **interfase** (G1, S e G2) e **fase M**, que inclui a mitose e a citocinese."),
        Block::new(BlockKind::H2, "Fases da mitose"),
        Block::new(BlockKind::Numbered, "**Prófase** — a cromatina se condensa e o fuso mitótico começa a se formar."),
        Block::new(BlockKind::Numbered, "**Metáfase** — os cromossomos se alinham na ==placa equatorial, presos ao fuso pelos cinetócoros==."),
        Block::new(BlockKind::Numbered, "**Anáfase** — as cromátides-irmãs se separam e migram para polos opostos."),
        Block::new(BlockKind::Numbered, "**Telófase** — os envoltórios nucleares se refazem; segue-se a citocinese."),
        Block::new(BlockKind::Callout, "Mnemônico *PMAT*. Compare com a divisão reducional em [[Meiose]]."),
        Block::new(BlockKind::H2, "Regulação e checkpoints"),
        Block::new(BlockKind::Paragraph, "Os pontos de checagem G1/S, G2/M e do fuso impedem que a célula avance com DNA danificado ou cromossomos mal ligados. Falhas nesses controles aparecem em [[Checkpoints e câncer]]."),
        Block::new(BlockKind::Table, ""),
    ];
    blocks[9].rows = vec![
        vec!["Checkpoint".into(), "O que verifica".into(), "Proteína-chave".into()],
        vec!["G1/S".into(), "Dano no DNA, tamanho celular".into(), "p53, Rb".into()],
        vec!["G2/M".into(), "Replicação completa".into(), "CDK1–ciclina B".into()],
        vec!["Fuso (metáfase)".into(), "Cinetócoros ligados".into(), "Mad2, APC/C".into()],
    ];
    db.save_blocks(ciclo, "Ciclo celular e mitose", &blocks)?;
    db.save_blocks(meiose, "Meiose", &[
        Block::new(BlockKind::Paragraph, "Divisão **reducional**: uma célula diploide gera quatro haploides. O *crossing-over* na prófase I aumenta a variabilidade. Veja também [[Ciclo celular e mitose]]."),
        Block::new(BlockKind::Bullet, "Meiose I separa homólogos."),
        Block::new(BlockKind::Bullet, "Meiose II separa cromátides-irmãs (como a mitose)."),
    ])?;
    db.save_blocks(check, "Checkpoints e câncer", &[Block::new(BlockKind::Paragraph, "Mutações em **p53** (o \"guardião do genoma\") estão presentes em cerca de metade dos tumores humanos. Ver [[Ciclo celular e mitose]].")])?;
    db.save_blocks(membrana, "Membrana plasmática", &[Block::new(BlockKind::Paragraph, "Modelo do **mosaico fluido**: bicamada fosfolipídica com proteínas integrais e periféricas; o colesterol modula a fluidez.")])?;
    db.save_blocks(mito, "Mitocôndria", &[Block::new(BlockKind::Paragraph, "Organela da **respiração celular**: ciclo de Krebs na matriz e cadeia transportadora de elétrons na membrana interna. Tem DNA próprio, de herança materna.")])?;
    let plexo = pg(anat, None, "Plexo braquial")?;
    db.save_blocks(plexo, "Plexo braquial", &[
        Block::new(BlockKind::Paragraph, "Formado pelos ramos anteriores de **C5–T1**. Organização: raízes, troncos, divisões, fascículos e ramos terminais."),
        Block::new(BlockKind::Quote, "Randy Travis Drinks Cold Beer — raízes, troncos, divisões, fascículos, ramos."),
    ])?;
    let beta = pg(farm, None, "Betabloqueadores")?;
    db.save_blocks(beta, "Betabloqueadores", &[Block::new(BlockKind::Paragraph, "Antagonistas β-adrenérgicos. **β₁-seletivos**: atenolol, metoprolol, bisoprolol. **Não seletivos**: propranolol, nadolol. Cuidado em asma (broncoespasmo).")])?;
    let ieca = pg(farm, None, "Inibidores da ECA")?;
    db.save_blocks(ieca, "Inibidores da ECA", &[Block::new(BlockKind::Paragraph, "Bloqueiam a conversão de angiotensina I em angiotensina II e reduzem a degradação de bradicinina (tosse, angioedema). Contraindicados na gestação.")])?;
    let cc = pg(dir, None, "Controle de constitucionalidade")?;
    db.save_blocks(cc, "Controle de constitucionalidade", &[Block::new(BlockKind::Paragraph, "**Difuso** (qualquer juiz, caso concreto, efeito *inter partes*) e **concentrado** (STF, ADI/ADC/ADPF, efeito *erga omnes* e vinculante).")])?;

    // Notas.
    let notes: Vec<(i64, Note, Option<i64>)> = vec![
        (antihip, basic("Qual é o mecanismo de ação dos betabloqueadores no tratamento da hipertensão?", "Antagonismo competitivo dos receptores β₁-adrenérgicos: reduz a frequência e a contratilidade cardíacas e a liberação de renina.", "Ex.: atenolol e metoprolol são β₁-seletivos; propranolol não é seletivo.", "prova-p2"), Some(beta)),
        (antihip, cloze("Os inibidores da ECA bloqueiam a conversão de angiotensina I em {{c1::angiotensina II}}.", "Também reduzem a degradação da bradicinina, o que explica parte dos efeitos adversos.", "prova-p2"), Some(ieca)),
        (antihip, basic("Qual efeito adverso característico dos inibidores da ECA está ligado ao acúmulo de bradicinina?", "Tosse seca persistente (e, raramente, angioedema).", "Nesse caso, trocar por um bloqueador do receptor AT₁ (BRA), como a losartana.", "prova-p2"), Some(ieca)),
        (antihip, basic("Contraindicação absoluta de betabloqueador não seletivo?", "Asma (risco de broncoespasmo).", "", ""), Some(beta)),
        (antihip, basic("Hidroclorotiazida: principais distúrbios eletrolíticos?", "Hipocalemia, hiponatremia, hipercalcemia e hiperuricemia.", "", ""), None),
        (antihip, cloze("Anlodipino é um bloqueador de canais de cálcio do tipo {{c1::di-hidropiridínico}}, com efeito predominante {{c2::vascular}}.", "", ""), None),
        (antiarr, basic("Classe da amiodarona (Vaughan Williams)?", "Classe III (bloqueio de canais de K⁺), com ações de todas as classes.", "", ""), None),
        (antiarr, basic("Por que o verapamil é evitado com betabloqueador?", "Ambos deprimem o nó AV e a contratilidade: risco de bradicardia e bloqueio AV.", "", ""), None),
        (cardio, basic("Mecanismo da digoxina na insuficiência cardíaca?", "Inibe a Na⁺/K⁺-ATPase, aumentando o Ca²⁺ intracelular (inotropismo positivo) e o tônus vagal.", "", ""), None),
        (cardio, basic("Antídoto da intoxicação por heparina?", "Sulfato de protamina.", "", ""), None),
        (cardio, basic("Espironolactona e ginecomastia: por quê?", "Antagonismo de receptores androgênicos (além do mineralocorticoide).", "", ""), None),
        (adren, basic("Mecanismo da clonidina?", "Agonista α₂ central: reduz o tônus simpático.", "", ""), None),
        (colin, basic("Atropina: receptor e efeito na frequência cardíaca?", "Antagonista muscarínico; aumenta a FC (bloqueia o vago).", "", ""), None),
        (colin, basic("Neostigmina: uso clínico principal?", "Reversão do bloqueio neuromuscular e miastenia gravis.", "", ""), None),
        (antimic, basic("Mecanismo dos betalactâmicos?", "Inibem a síntese da parede celular ligando-se às PBPs (transpeptidases).", "", ""), None),
        (antimic, cloze("Aminoglicosídeos causam {{c1::nefrotoxicidade}} e {{c2::ototoxicidade}}.", "", "prova-p2"), None),
        (f, basic("Sinal clássico de toxicidade por lítio?", "Tremor grosseiro, ataxia e confusão; janela terapêutica estreita.", "", ""), None),
        (divisao, cloze("Na {{c1::metáfase}}, os cromossomos se alinham na {{c2::placa equatorial}}, presos ao fuso pelos {{c3::cinetócoros::estrutura proteica}}.", "A placa equatorial é um plano imaginário; os cinetócoros se formam nos centrômeros.", "ciclo-celular prova-p2"), Some(ciclo)),
        (divisao, cloze("A síntese de DNA ocorre na fase {{c1::S}} da interfase.", "", "ciclo-celular"), Some(ciclo)),
        (divisao, basic("O que acontece na anáfase?", "As cromátides-irmãs se separam e migram para polos opostos.", "", "ciclo-celular"), Some(ciclo)),
        (divisao, basic("Qual a principal diferença entre mitose e meiose?", "Mitose gera 2 células idênticas (2n); meiose gera 4 haploides (n) e variabilidade.", "", ""), Some(meiose)),
        (b, basic("Qual proteína é chamada de \"guardiã do genoma\"?", "p53.", "", ""), Some(check)),
        (b, cloze("A respiração celular ocorre principalmente na {{c1::mitocôndria}}, cuja membrana {{c2::interna}} abriga a cadeia transportadora de elétrons.", "", ""), Some(mito)),
        (b, basic("Modelo que descreve a membrana plasmática?", "Mosaico fluido.", "", ""), Some(membrana)),
        (membros, basic("Raízes que formam o plexo braquial?", "Ramos anteriores de C5 a T1.", "", ""), Some(plexo)),
        (membros, basic("Lesão do nervo radial: sinal clássico?", "Mão caída (perda da extensão do punho).", "", ""), Some(plexo)),
        (membros, basic("Nervo lesado na \"mão em garra\"?", "Nervo ulnar.", "", ""), Some(plexo)),
        (membros, basic("Músculos do manguito rotador?", "Supraespinal, infraespinal, redondo menor e subescapular.", "", ""), None),
        (a, basic("Qual artéria irriga a maior parte do miocárdio do ventrículo esquerdo?", "Artéria descendente anterior (ramo da coronária esquerda).", "", ""), None),
        (a, cloze("O nervo {{c1::frênico}} inerva o diafragma (raízes {{c2::C3–C5}}).", "", ""), None),
        (d, basic("Quem pode propor ADI?", "Os legitimados do art. 103 da CF (Presidente, Mesas, PGR, OAB, partidos, confederações…).", "", ""), Some(cc)),
        (d, cloze("O controle {{c1::difuso}} é feito por qualquer juiz no caso concreto; o {{c2::concentrado}}, pelo STF em abstrato.", "", ""), Some(cc)),
        (d, basic("Efeitos da decisão em ADI?", "Erga omnes, vinculante e, em regra, ex tunc.", "", ""), Some(cc)),
        (d, basic("O que é cláusula pétrea?", "Matéria que não pode ser abolida por emenda (art. 60, §4º).", "", ""), None),
    ];

    // Histórico simulado: cada cartão começa em um dia do último ano e é revisado
    // quando vence, com acerto sorteado pela recuperabilidade.
    let clock = Clock { rollover_hour: 4 };
    let now = srs::now_ms();
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let cfg = PresetConfig::default();
    let (fsrs_m, decay) = srs::fsrs_for(&[]);
    let zero = |_: i64| 0usize;
    for (deck, mut note, page) in notes {
        note.source_page = page;
        db.add_note(&mut note, deck)?;
        for mut card in db.note_cards(note.id)? {
            if rng.random_bool(0.18) {
                continue; // fica como novo
            }
            let mut t = now - rng.random_range(5..300) * DAY_MS + rng.random_range(0..12) * 3_600_000;
            let mut s = Sched::default();
            let mut reps = 0;
            while t < now && reps < 40 {
                let r = srs::retrievability(&s, decay, &clock, t).unwrap_or(0.85);
                let p_ok = if s.state == State::Review { r } else { 0.78 };
                let rating = if rng.random_bool(p_ok.clamp(0.05, 0.99) as f64) {
                    let x: f64 = rng.random();
                    if x < 0.12 { 2 } else if x < 0.88 { 3 } else { 4 }
                } else {
                    1
                };
                let ctx = srs::Ctx { cfg: &cfg, fsrs: &fsrs_m, decay, clock, now: t, seed: card.id as u64 ^ reps, due_on_day: &zero, filtered: false };
                let out = srs::answer(&s, rating, &ctx);
                db.add_rev(&Rev {
                    id: 0,
                    card_id: card.id,
                    at: t,
                    rating,
                    kind: out.kind,
                    elapsed_days: out.elapsed_days,
                    scheduled_days: out.scheduled_days,
                    duration_ms: rng.random_range(3_000..22_000),
                    state_before: s.state,
                    state_after: out.sched.state,
                    s_before: s.stability,
                    s_after: out.sched.stability,
                    d_before: s.difficulty,
                    d_after: out.sched.difficulty,
                    retrievability: out.retrievability,
                    next_due: out.sched.due,
                    algorithm: "fsrs-6".into(),
                    preset_id: 1,
                })?;
                s = out.sched;
                reps += 1;
                // Atrasos ocasionais, como na vida real.
                let late = if rng.random_bool(0.2) { rng.random_range(0..3) * DAY_MS } else { 0 };
                t = s.due + late + rng.random_range(0..6) * 600_000;
            }
            card.s = s;
            db.save_card(&card)?;
        }
    }
    Ok(())
}
