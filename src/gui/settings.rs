//! Configurações (aparência, estudo, dados e backup) e o conteúdo de exemplo.

use super::theme::*;
use super::widgets::*;
use super::App;
use crate::{tr, tr_n};
use crate::cards::NoteKind;
use crate::db::{Block, BlockKind, Db, Note, Rev};
use crate::srs::{self, Clock, DAY_MS, PresetConfig, Sched, State};
use eframe::egui::{self, Sense, Ui, vec2};
use rand::{Rng, SeedableRng};

pub fn ui(app: &mut App, ui: &mut Ui) {
    let p = pal();
    super::header(ui, &tr!("common.settings"), &tr!("settings.subtitle", version = super::VERSION, path = app.db.root.display()), |_| {});
    let area = ui.available_rect_before_wrap();
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(28.0, 20.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut cu, |ui| {
        ui.set_max_width(760.0);
        ui.spacing_mut().item_spacing.y = 16.0;
        section(ui, &tr!("settings.appearance"), |ui| {
            row(ui, &tr!("app.language"), |ui| {
                let cur = current_language_name(app);
                let lresp = select_field(ui, "settings_lang", &cur, None, 220.0, 32.0, |ui| language_menu(app, ui));
                lresp.on_hover_text(tr!("settings.language_hint"));
            });
            row(ui, &tr!("settings.theme"), |ui| {
                let mut sel = if app.cfg.dark { 0 } else { 1 };
                let (dark, light) = (tr!("settings.theme_dark"), tr!("settings.theme_light"));
                if segmented(ui, &[dark.as_str(), light.as_str()], &mut sel, 28.0, false) {
                    app.cfg.dark = sel == 0;
                    app.save_settings();
                    apply(ui.ctx(), app.cfg.dark);
                }
            });
            row(ui, &tr!("settings.reading_width"), |ui| {
                if ui.add(egui::Slider::new(&mut app.cfg.reading_width, 520.0..=900.0).suffix(" px").step_by(20.0)).changed() {
                    app.save_settings();
                }
            });
            row(ui, &tr!("settings.sidebar_collapsed"), |ui| {
                if toggle(ui, &mut app.cfg.sidebar_collapsed, &tr!("settings.sidebar_collapsed")).clicked() {
                    app.save_settings();
                }
            });
        });
        section(ui, &tr!("settings.study"), |ui| {
            row(ui, &tr!("settings.rollover"), |ui| {
                let mut h = app.cfg.rollover_hour;
                if ui.add(egui::DragValue::new(&mut h).range(0..=12).suffix(":00")).changed() {
                    app.cfg.rollover_hour = h;
                    app.save_settings();
                    app.changed();
                }
            });
            row(ui, &tr!("settings.answer_cap"), |ui| {
                let mut s = app.cfg.answer_cap_secs;
                if ui.add(egui::DragValue::new(&mut s).range(10..=600).suffix(" s")).changed() {
                    app.cfg.answer_cap_secs = s;
                    app.save_settings();
                }
            });
            row(ui, &tr!("settings.presets"), |ui| {
                if button(ui, &tr!("settings.open_presets"), Btn::Secondary, 32.0).clicked() {
                    let id = app.presets.first().map(|p| p.id).unwrap_or(1);
                    app.go(super::Screen::Preset(id));
                }
            });
        });
        super::ai::settings_section(app, ui);
        section(ui, &tr!("settings.data"), |ui| {
            row(ui, &tr!("settings.auto_backup"), |ui| {
                if toggle(ui, &mut app.cfg.auto_backup, &tr!("settings.auto_backup_short")).clicked() {
                    app.save_settings();
                }
            });
            row(ui, &tr!("settings.last_backup", when = if app.cfg.last_backup > 0 { ago(app.cfg.last_backup) } else { tr!("settings.never") }), |ui| {
                if button(ui, &tr!("settings.backup_now"), Btn::Secondary, 32.0).clicked() {
                    app.maybe_backup(true);
                    app.toast(tr!("toast.backup_done"));
                }
            });
            row(ui, &tr!("settings.restore"), |ui| {
                if button(ui, &tr!("settings.restore_button"), Btn::Secondary, 32.0).clicked() {
                    let dir = app.db.root.join("backups");
                    if let Some(path) = rfd::FileDialog::new().set_directory(&dir).add_filter(tr!("settings.db_filter"), &["db"]).pick_file() {
                        match std::fs::write(app.db.root.join("restore.pending"), path.to_string_lossy().as_bytes()) {
                            Ok(()) => app.toast(tr!("toast.restore_pending")),
                            Err(e) => app.error(e),
                        }
                    }
                }
            });
            row(ui, &tr!("settings.data_folder"), |ui| {
                if button(ui, &tr!("settings.open_folder"), Btn::Secondary, 32.0).clicked() {
                    open_folder(&app.db.root);
                }
            });
            row(ui, &tr!("settings.orphans"), |ui| {
                if button(ui, &tr!("settings.clean_orphans"), Btn::Secondary, 32.0).clicked() {
                    match app.db.orphan_media() {
                        Ok(list) => {
                            for f in &list {
                                let _ = std::fs::remove_file(app.db.media_dir().join(f));
                            }
                            app.toast(tr_n!("toast.files_removed", list.len()));
                        }
                        Err(e) => app.error(e),
                    }
                }
            });
            row(ui, &tr!("settings.integrity"), |ui| {
                if button(ui, &tr!("settings.check"), Btn::Secondary, 32.0).clicked() {
                    let r: String = app.db.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap_or_else(|e| e.to_string());
                    if r == "ok" {
                        app.toast(tr!("toast.db_ok"));
                    } else {
                        app.error(tr!("toast.db_problem", detail = r));
                    }
                }
            });
            if app.subjects.is_empty() {
                row(ui, &tr!("settings.sample"), |ui| {
                    if button(ui, &tr!("dash.load_sample"), Btn::Secondary, 32.0).clicked() {
                        match seed_demo(&app.db) {
                            Ok(()) => {
                                app.reload();
                                app.toast(tr!("toast.sample_short"));
                            }
                            Err(e) => app.error(e),
                        }
                    }
                });
            }
        });
        section(ui, &tr!("settings.about"), |ui| {
            para(ui, &tr!("settings.about_text"), serif(13.0, W::Regular), p.text2);
            para(ui, &tr!("settings.fonts"), serif(12.0, W::Italic), p.muted);
        });
    });
}

/// Nome do idioma atual (ou "Automático").
fn current_language_name(app: &App) -> String {
    match &app.cfg.lang {
        None => tr!("app.language_auto"),
        Some(code) => crate::i18n::languages().into_iter().find(|l| &l.code == code).map(|l| l.name).unwrap_or_else(|| code.clone()),
    }
}

/// Lista de idiomas (Configurações e o seletor da barra de título).
pub fn language_menu(app: &mut App, ui: &mut Ui) {
    let mut pick: Option<Option<String>> = None;
    if menu_item(ui, &tr!("app.language_auto"), app.cfg.lang.is_none()).clicked() {
        pick = Some(None);
    }
    menu_sep(ui);
    for l in crate::i18n::languages() {
        if menu_item(ui, &l.name, app.cfg.lang.as_deref() == Some(l.code.as_str())).clicked() {
            pick = Some(Some(l.code));
        }
    }
    if let Some(lang) = pick {
        crate::i18n::set_language(lang.as_deref());
        app.cfg.lang = lang;
        app.save_settings();
        // Rótulos guardados em caches (gráficos, contagens) são refeitos no idioma novo.
        app.data_version += 1;
    }
}

pub(super) fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    card_frame().inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 10.0;
        label(ui, title, serif(16.0, W::Semi), p.text);
        add(ui);
    });
}

pub(super) fn row(ui: &mut Ui, label_s: &str, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    ui.horizontal(|ui| {
        ui.set_min_height(34.0);
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w * 0.55, 20.0), Sense::hover());
        text(ui.painter(), r.left_center(), egui::Align2::LEFT_CENTER, label_s, serif(13.5, W::Regular), p.text2, r.width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

pub(super) fn open_folder(path: &std::path::Path) {
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
    // O exemplo sai no idioma da interface: inglês, salvo se ela estiver em português.
    let en = !crate::i18n::current().starts_with("pt");
    let t = |en_text: &'static str, pt_text: &'static str| if en { en_text } else { pt_text };
    let anat = db.add_subject(t("Anatomy", "Anatomia"), 0)?;
    let farm = db.add_subject(t("Pharmacology", "Farmacologia"), 1)?;
    let bio = db.add_subject(t("Cell Biology", "Biologia Celular"), 2)?;
    let dir = db.add_subject(t("Constitutional Law", "Direito Constitucional"), 3)?;
    let decks = db.list_decks()?;
    let root = |s: i64| decks.iter().find(|d| d.subject_id == s).map(|d| d.id).unwrap();
    let med = db.add_preset(t("Medicine", "Medicina"), &PresetConfig::default(), &[])?;
    for s in [anat, farm, bio] {
        if let Some(mut d) = db.list_decks()?.into_iter().find(|d| d.id == root(s)) {
            d.preset_id = med;
            db.update_deck(&d)?;
        }
    }

    // Baralhos de Farmacologia (como no design).
    let f = root(farm);
    let cardio = db.add_deck(farm, Some(f), t("Cardiovascular", "Cardiovascular"))?;
    let antihip = db.add_deck(farm, Some(cardio), t("Antihypertensives", "Anti-hipertensivos"))?;
    let antiarr = db.add_deck(farm, Some(cardio), t("Antiarrhythmics", "Antiarrítmicos"))?;
    let sna = db.add_deck(farm, Some(f), t("Autonomic nervous system", "Sistema nervoso autônomo"))?;
    let colin = db.add_deck(farm, Some(sna), t("Cholinergics", "Colinérgicos"))?;
    let adren = db.add_deck(farm, Some(sna), t("Adrenergics", "Adrenérgicos"))?;
    let antimic = db.add_deck(farm, Some(f), t("Antimicrobials", "Antimicrobianos"))?;
    let _pk = db.add_deck(farm, Some(f), t("Pharmacokinetics", "Farmacocinética"))?;
    let b = root(bio);
    let divisao = db.add_deck(bio, Some(b), t("Cell division", "Divisão celular"))?;
    let a = root(anat);
    let membros = db.add_deck(anat, Some(a), t("Upper limb", "Membro superior"))?;
    let d = root(dir);

    // Páginas de Biologia Celular.
    let pg = |s: i64, parent: Option<i64>, t: &str| db.add_page(s, parent, t);
    let membrana = pg(bio, None, t("Plasma membrane", "Membrana plasmática"))?;
    let organelas = pg(bio, None, t("Organelles", "Organelas"))?;
    let mito = pg(bio, Some(organelas), t("Mitochondria", "Mitocôndria"))?;
    let _ret = pg(bio, Some(organelas), t("Endoplasmic reticulum", "Retículo endoplasmático"))?;
    let ciclo = pg(bio, None, t("Cell cycle and mitosis", "Ciclo celular e mitose"))?;
    let meiose = pg(bio, Some(ciclo), t("Meiosis", "Meiose"))?;
    let check = pg(bio, Some(ciclo), t("Checkpoints and cancer", "Checkpoints e câncer"))?;
    let _apop = pg(bio, None, t("Apoptosis", "Apoptose"))?;
    let _ex = pg(bio, None, t("Exercises — division", "Exercícios — divisão"))?;
    let mut blocks = vec![
        Block::new(BlockKind::Paragraph, t("The cell cycle is the sequence of events in which a cell duplicates its contents and divides in two. It is split into **interphase** (G1, S and G2) and the **M phase**, which includes mitosis and cytokinesis.", "O ciclo celular é a sequência de eventos pela qual uma célula duplica seu conteúdo e se divide em duas. Ele se divide em **interfase** (G1, S e G2) e **fase M**, que inclui a mitose e a citocinese.")),
        Block::new(BlockKind::H2, t("Phases of mitosis", "Fases da mitose")),
        Block::new(BlockKind::Numbered, t("**Prophase** — chromatin condenses and the mitotic spindle starts to form.", "**Prófase** — a cromatina se condensa e o fuso mitótico começa a se formar.")),
        Block::new(BlockKind::Numbered, t("**Metaphase** — chromosomes line up on the ==metaphase plate, attached to the spindle by their kinetochores==.", "**Metáfase** — os cromossomos se alinham na ==placa equatorial, presos ao fuso pelos cinetócoros==.")),
        Block::new(BlockKind::Numbered, t("**Anaphase** — sister chromatids separate and move to opposite poles.", "**Anáfase** — as cromátides-irmãs se separam e migram para polos opostos.")),
        Block::new(BlockKind::Numbered, t("**Telophase** — the nuclear envelopes re-form; cytokinesis follows.", "**Telófase** — os envoltórios nucleares se refazem; segue-se a citocinese.")),
        Block::new(BlockKind::Callout, t("Mnemonic *PMAT*. Compare with the reductional division in [[Meiosis]].", "Mnemônico *PMAT*. Compare com a divisão reducional em [[Meiose]].")),
        Block::new(BlockKind::H2, t("Regulation and checkpoints", "Regulação e checkpoints")),
        Block::new(BlockKind::Paragraph, t("The G1/S, G2/M and spindle checkpoints stop the cell from moving on with damaged DNA or badly attached chromosomes. Failures in these controls show up in [[Checkpoints and cancer]].", "Os pontos de checagem G1/S, G2/M e do fuso impedem que a célula avance com DNA danificado ou cromossomos mal ligados. Falhas nesses controles aparecem em [[Checkpoints e câncer]].")),
        Block::new(BlockKind::Table, ""),
    ];
    blocks[9].rows = vec![
        vec![t("Checkpoint", "Checkpoint").into(), t("What it checks", "O que verifica").into(), t("Key protein", "Proteína-chave").into()],
        vec!["G1/S".into(), t("DNA damage, cell size", "Dano no DNA, tamanho celular").into(), t("p53, Rb", "p53, Rb").into()],
        vec!["G2/M".into(), t("Complete replication", "Replicação completa").into(), t("CDK1–cyclin B", "CDK1–ciclina B").into()],
        vec![t("Spindle (metaphase)", "Fuso (metáfase)").into(), t("Kinetochores attached", "Cinetócoros ligados").into(), t("Mad2, APC/C", "Mad2, APC/C").into()],
    ];
    db.save_blocks(ciclo, t("Cell cycle and mitosis", "Ciclo celular e mitose"), &blocks)?;
    db.save_blocks(meiose, t("Meiosis", "Meiose"), &[
        Block::new(BlockKind::Paragraph, t("A **reductional** division: one diploid cell gives four haploid ones. *Crossing-over* in prophase I increases variability. See also [[Cell cycle and mitosis]].", "Divisão **reducional**: uma célula diploide gera quatro haploides. O *crossing-over* na prófase I aumenta a variabilidade. Veja também [[Ciclo celular e mitose]].")),
        Block::new(BlockKind::Bullet, t("Meiosis I separates homologues.", "Meiose I separa homólogos.")),
        Block::new(BlockKind::Bullet, t("Meiosis II separates sister chromatids (like mitosis).", "Meiose II separa cromátides-irmãs (como a mitose).")),
    ])?;
    db.save_blocks(check, t("Checkpoints and cancer", "Checkpoints e câncer"), &[Block::new(BlockKind::Paragraph, t("Mutations in **p53** (the \"guardian of the genome\") are found in about half of human tumours. See [[Cell cycle and mitosis]].", "Mutações em **p53** (o \"guardião do genoma\") estão presentes em cerca de metade dos tumores humanos. Ver [[Ciclo celular e mitose]]."))])?;
    db.save_blocks(membrana, t("Plasma membrane", "Membrana plasmática"), &[Block::new(BlockKind::Paragraph, t("The **fluid mosaic** model: a phospholipid bilayer with integral and peripheral proteins; cholesterol modulates fluidity.", "Modelo do **mosaico fluido**: bicamada fosfolipídica com proteínas integrais e periféricas; o colesterol modula a fluidez."))])?;
    db.save_blocks(mito, t("Mitochondria", "Mitocôndria"), &[Block::new(BlockKind::Paragraph, t("The organelle of **cellular respiration**: the Krebs cycle in the matrix and the electron transport chain in the inner membrane. It has its own DNA, inherited from the mother.", "Organela da **respiração celular**: ciclo de Krebs na matriz e cadeia transportadora de elétrons na membrana interna. Tem DNA próprio, de herança materna."))])?;
    let plexo = pg(anat, None, t("Brachial plexus", "Plexo braquial"))?;
    db.save_blocks(plexo, t("Brachial plexus", "Plexo braquial"), &[
        Block::new(BlockKind::Paragraph, t("Formed by the anterior rami of **C5–T1**. Layout: roots, trunks, divisions, cords and terminal branches.", "Formado pelos ramos anteriores de **C5–T1**. Organização: raízes, troncos, divisões, fascículos e ramos terminais.")),
        Block::new(BlockKind::Quote, t("Randy Travis Drinks Cold Beer — roots, trunks, divisions, cords, branches.", "Randy Travis Drinks Cold Beer — raízes, troncos, divisões, fascículos, ramos.")),
    ])?;
    let beta = pg(farm, None, t("Beta blockers", "Betabloqueadores"))?;
    db.save_blocks(beta, t("Beta blockers", "Betabloqueadores"), &[Block::new(BlockKind::Paragraph, t("β-adrenergic antagonists. **β₁-selective**: atenolol, metoprolol, bisoprolol. **Non-selective**: propranolol, nadolol. Careful in asthma (bronchospasm).", "Antagonistas β-adrenérgicos. **β₁-seletivos**: atenolol, metoprolol, bisoprolol. **Não seletivos**: propranolol, nadolol. Cuidado em asma (broncoespasmo)."))])?;
    let ieca = pg(farm, None, t("ACE inhibitors", "Inibidores da ECA"))?;
    db.save_blocks(ieca, t("ACE inhibitors", "Inibidores da ECA"), &[Block::new(BlockKind::Paragraph, t("They block the conversion of angiotensin I into angiotensin II and reduce the breakdown of bradykinin (cough, angioedema). Contraindicated in pregnancy.", "Bloqueiam a conversão de angiotensina I em angiotensina II e reduzem a degradação de bradicinina (tosse, angioedema). Contraindicados na gestação."))])?;
    let cc = pg(dir, None, t("Judicial review", "Controle de constitucionalidade"))?;
    db.save_blocks(cc, t("Judicial review", "Controle de constitucionalidade"), &[Block::new(BlockKind::Paragraph, t("Established in **Marbury v. Madison** (1803): federal courts may strike down laws that conflict with the Constitution. Any court can rule on constitutionality in a *concrete case*; the **Supreme Court** has the final word.", "**Difuso** (qualquer juiz, caso concreto, efeito *inter partes*) e **concentrado** (STF, ADI/ADC/ADPF, efeito *erga omnes* e vinculante)."))])?;

    // Notas.
    let notes: Vec<(i64, Note, Option<i64>)> = vec![
        (antihip, basic(t("What is the mechanism of action of beta blockers in treating hypertension?", "Qual é o mecanismo de ação dos betabloqueadores no tratamento da hipertensão?"), t("Competitive antagonism of β₁-adrenergic receptors: lowers heart rate, contractility and renin release.", "Antagonismo competitivo dos receptores β₁-adrenérgicos: reduz a frequência e a contratilidade cardíacas e a liberação de renina."), t("E.g. atenolol and metoprolol are β₁-selective; propranolol is non-selective.", "Ex.: atenolol e metoprolol são β₁-seletivos; propranolol não é seletivo."), t("exam-2", "prova-p2")), Some(beta)),
        (antihip, cloze(t("ACE inhibitors block the conversion of angiotensin I into {{c1::angiotensin II}}.", "Os inibidores da ECA bloqueiam a conversão de angiotensina I em {{c1::angiotensina II}}."), t("They also reduce the breakdown of bradykinin, which explains some of the side effects.", "Também reduzem a degradação da bradicinina, o que explica parte dos efeitos adversos."), t("exam-2", "prova-p2")), Some(ieca)),
        (antihip, basic(t("Which typical side effect of ACE inhibitors is linked to bradykinin build-up?", "Qual efeito adverso característico dos inibidores da ECA está ligado ao acúmulo de bradicinina?"), t("A persistent dry cough (and, rarely, angioedema).", "Tosse seca persistente (e, raramente, angioedema)."), t("In that case, switch to an AT₁ receptor blocker (ARB) such as losartan.", "Nesse caso, trocar por um bloqueador do receptor AT₁ (BRA), como a losartana."), t("exam-2", "prova-p2")), Some(ieca)),
        (antihip, basic(t("Absolute contraindication for a non-selective beta blocker?", "Contraindicação absoluta de betabloqueador não seletivo?"), t("Asthma (risk of bronchospasm).", "Asma (risco de broncoespasmo)."), "", ""), Some(beta)),
        (antihip, basic(t("Hydrochlorothiazide: main electrolyte disturbances?", "Hidroclorotiazida: principais distúrbios eletrolíticos?"), t("Hypokalaemia, hyponatraemia, hypercalcaemia and hyperuricaemia.", "Hipocalemia, hiponatremia, hipercalcemia e hiperuricemia."), "", ""), None),
        (antihip, cloze(t("Amlodipine is a {{c1::dihydropyridine}} calcium channel blocker with a mainly {{c2::vascular}} effect.", "Anlodipino é um bloqueador de canais de cálcio do tipo {{c1::di-hidropiridínico}}, com efeito predominante {{c2::vascular}}."), "", ""), None),
        (antiarr, basic(t("Amiodarone's class (Vaughan Williams)?", "Classe da amiodarona (Vaughan Williams)?"), t("Class III (K⁺ channel blockade), with actions of every class.", "Classe III (bloqueio de canais de K⁺), com ações de todas as classes."), "", ""), None),
        (antiarr, basic(t("Why is verapamil avoided with a beta blocker?", "Por que o verapamil é evitado com betabloqueador?"), t("Both depress the AV node and contractility: risk of bradycardia and AV block.", "Ambos deprimem o nó AV e a contratilidade: risco de bradicardia e bloqueio AV."), "", ""), None),
        (cardio, basic(t("Mechanism of digoxin in heart failure?", "Mecanismo da digoxina na insuficiência cardíaca?"), t("It inhibits Na⁺/K⁺-ATPase, raising intracellular Ca²⁺ (positive inotropy) and vagal tone.", "Inibe a Na⁺/K⁺-ATPase, aumentando o Ca²⁺ intracelular (inotropismo positivo) e o tônus vagal."), "", ""), None),
        (cardio, basic(t("Antidote for heparin overdose?", "Antídoto da intoxicação por heparina?"), t("Protamine sulfate.", "Sulfato de protamina."), "", ""), None),
        (cardio, basic(t("Spironolactone and gynaecomastia: why?", "Espironolactona e ginecomastia: por quê?"), t("Androgen receptor antagonism (besides the mineralocorticoid one).", "Antagonismo de receptores androgênicos (além do mineralocorticoide)."), "", ""), None),
        (adren, basic(t("Mechanism of clonidine?", "Mecanismo da clonidina?"), t("Central α₂ agonist: lowers sympathetic tone.", "Agonista α₂ central: reduz o tônus simpático."), "", ""), None),
        (colin, basic(t("Atropine: receptor and effect on heart rate?", "Atropina: receptor e efeito na frequência cardíaca?"), t("Muscarinic antagonist; raises heart rate (blocks the vagus).", "Antagonista muscarínico; aumenta a FC (bloqueia o vago)."), "", ""), None),
        (colin, basic(t("Neostigmine: main clinical use?", "Neostigmina: uso clínico principal?"), t("Reversing neuromuscular blockade, and myasthenia gravis.", "Reversão do bloqueio neuromuscular e miastenia gravis."), "", ""), None),
        (antimic, basic(t("Mechanism of beta-lactams?", "Mecanismo dos betalactâmicos?"), t("They inhibit cell wall synthesis by binding PBPs (transpeptidases).", "Inibem a síntese da parede celular ligando-se às PBPs (transpeptidases)."), "", ""), None),
        (antimic, cloze(t("Aminoglycosides cause {{c1::nephrotoxicity}} and {{c2::ototoxicity}}.", "Aminoglicosídeos causam {{c1::nefrotoxicidade}} e {{c2::ototoxicidade}}."), "", t("exam-2", "prova-p2")), None),
        (f, basic(t("Classic sign of lithium toxicity?", "Sinal clássico de toxicidade por lítio?"), t("Coarse tremor, ataxia and confusion; narrow therapeutic window.", "Tremor grosseiro, ataxia e confusão; janela terapêutica estreita."), "", ""), None),
        (divisao, cloze(t("In {{c1::metaphase}}, chromosomes line up on the {{c2::metaphase plate}}, attached to the spindle by their {{c3::kinetochores::protein structure}}.", "Na {{c1::metáfase}}, os cromossomos se alinham na {{c2::placa equatorial}}, presos ao fuso pelos {{c3::cinetócoros::estrutura proteica}}."), t("The metaphase plate is an imaginary plane; kinetochores form at the centromeres.", "A placa equatorial é um plano imaginário; os cinetócoros se formam nos centrômeros."), t("cell-cycle exam-2", "ciclo-celular prova-p2")), Some(ciclo)),
        (divisao, cloze(t("DNA synthesis happens in the {{c1::S}} phase of interphase.", "A síntese de DNA ocorre na fase {{c1::S}} da interfase."), "", t("cell-cycle", "ciclo-celular")), Some(ciclo)),
        (divisao, basic(t("What happens in anaphase?", "O que acontece na anáfase?"), t("Sister chromatids separate and move to opposite poles.", "As cromátides-irmãs se separam e migram para polos opostos."), "", t("cell-cycle", "ciclo-celular")), Some(ciclo)),
        (divisao, basic(t("What is the main difference between mitosis and meiosis?", "Qual a principal diferença entre mitose e meiose?"), t("Mitosis makes 2 identical cells (2n); meiosis makes 4 haploid ones (n) and variability.", "Mitose gera 2 células idênticas (2n); meiose gera 4 haploides (n) e variabilidade."), "", ""), Some(meiose)),
        (b, basic(t("Which protein is called the \"guardian of the genome\"?", "Qual proteína é chamada de \"guardiã do genoma\"?"), "p53.", "", ""), Some(check)),
        (b, cloze(t("Cellular respiration mainly happens in the {{c1::mitochondria}}, whose {{c2::inner}} membrane holds the electron transport chain.", "A respiração celular ocorre principalmente na {{c1::mitocôndria}}, cuja membrana {{c2::interna}} abriga a cadeia transportadora de elétrons."), "", ""), Some(mito)),
        (b, basic(t("Model that describes the plasma membrane?", "Modelo que descreve a membrana plasmática?"), t("Fluid mosaic.", "Mosaico fluido."), "", ""), Some(membrana)),
        (membros, basic(t("Roots that form the brachial plexus?", "Raízes que formam o plexo braquial?"), t("Anterior rami of C5 to T1.", "Ramos anteriores de C5 a T1."), "", ""), Some(plexo)),
        (membros, basic(t("Radial nerve injury: classic sign?", "Lesão do nervo radial: sinal clássico?"), t("Wrist drop (loss of wrist extension).", "Mão caída (perda da extensão do punho)."), "", ""), Some(plexo)),
        (membros, basic(t("Nerve injured in \"claw hand\"?", "Nervo lesado na \"mão em garra\"?"), t("Ulnar nerve.", "Nervo ulnar."), "", ""), Some(plexo)),
        (membros, basic(t("Rotator cuff muscles?", "Músculos do manguito rotador?"), t("Supraspinatus, infraspinatus, teres minor and subscapularis.", "Supraespinal, infraespinal, redondo menor e subescapular."), "", ""), None),
        (a, basic(t("Which artery supplies most of the left ventricular myocardium?", "Qual artéria irriga a maior parte do miocárdio do ventrículo esquerdo?"), t("The left anterior descending artery (a branch of the left coronary).", "Artéria descendente anterior (ramo da coronária esquerda)."), "", ""), None),
        (a, cloze(t("The {{c1::phrenic}} nerve supplies the diaphragm (roots {{c2::C3–C5}}).", "O nervo {{c1::frênico}} inerva o diafragma (raízes {{c2::C3–C5}})."), "", ""), None),
        (d, basic(t("Which case established judicial review in the US?", "Quem pode propor ADI?"), t("Marbury v. Madison (1803).", "Os legitimados do art. 103 da CF (Presidente, Mesas, PGR, OAB, partidos, confederações…)."), "", ""), Some(cc)),
        (d, cloze(t("The first {{c1::ten}} amendments to the US Constitution are known as the {{c2::Bill of Rights}}.", "O controle {{c1::difuso}} é feito por qualquer juiz no caso concreto; o {{c2::concentrado}}, pelo STF em abstrato."), "", ""), Some(cc)),
        (d, basic(t("How many justices sit on the US Supreme Court?", "Efeitos da decisão em ADI?"), t("Nine: a Chief Justice and eight Associate Justices.", "Erga omnes, vinculante e, em regra, ex tunc."), "", ""), Some(cc)),
        (d, basic(t("What does Article V of the US Constitution govern?", "O que é cláusula pétrea?"), t("How the Constitution is amended.", "Matéria que não pode ser abolida por emenda (art. 60, §4º)."), "", ""), None),
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
