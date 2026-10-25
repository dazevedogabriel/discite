//! Avaliação da IA: gera cartões para um conjunto fixo de páginas (português e
//! inglês, curtas e longas) e grava tudo num JSONL para conferir à mão se cada
//! cartão diz só o que está nas notas. Não roda no `cargo test` comum:
//!
//! ```text
//! DISCITE_EVAL_MODEL=<arquivo .gguf> DISCITE_EVAL_OUT=<saida.jsonl> \
//!     cargo test --release eval_cards -- --ignored --nocapture
//! ```

use super::engine::{self, Event, GenRequest};
use super::prompt::{self, Style};
use crate::db::{Block, BlockKind};

fn page(title: &str, blocks: &[(BlockKind, &str)]) -> (String, Vec<Block>) {
    (title.to_owned(), blocks.iter().map(|(k, t)| Block::new(*k, *t)).collect())
}

fn corpus() -> Vec<(String, Vec<Block>)> {
    use BlockKind::*;
    vec![
        page("Retina", &[
            (H2, "Introdução"),
            (Paragraph, "A retina é a camada interna e sensorial do globo ocular, localizada na parte posterior entre a coroide e o corpo vítreo. Funciona como o filme de uma máquina fotográfica: capta a luz e a transforma em impulsos elétricos levados ao cérebro pelo nervo óptico."),
            (H2, "Fotorreceptores"),
            (Bullet, "Bastonetes: visão em baixa luminosidade (escotópica); cerca de 120 milhões por olho; contêm rodopsina."),
            (Bullet, "Cones: visão de cores e acuidade (fotópica); cerca de 6 milhões por olho; três tipos (S, M e L)."),
            (Paragraph, "A fóvea, no centro da mácula, tem só cones e é a região de maior acuidade. O disco óptico, por onde sai o nervo óptico, não tem fotorreceptores e corresponde ao ponto cego."),
            (H2, "Doenças"),
            (Paragraph, "No descolamento de retina o paciente relata flashes de luz, moscas volantes e uma sombra em cortina; é urgência oftalmológica. A retinopatia diabética é a principal causa de cegueira em adultos em idade produtiva."),
        ]),
        page("Insuficiência cardíaca", &[
            (Paragraph, "Insuficiência cardíaca (IC) é a incapacidade do coração de bombear sangue suficiente para as necessidades do organismo, ou de fazê-lo só com pressões de enchimento elevadas."),
            (H3, "Classificação pela fração de ejeção"),
            (Bullet, "ICFEr: fração de ejeção reduzida, ≤ 40%."),
            (Bullet, "ICFEi: levemente reduzida, 41 a 49%."),
            (Bullet, "ICFEp: preservada, ≥ 50%."),
            (H3, "Tratamento da ICFEr"),
            (Paragraph, "Os quatro pilares que reduzem mortalidade são: IECA/BRA ou sacubitril-valsartana, betabloqueador (carvedilol, bisoprolol ou succinato de metoprolol), antagonista mineralocorticoide (espironolactona) e inibidor de SGLT2 (dapagliflozina ou empagliflozina). Diuréticos de alça aliviam a congestão, mas não reduzem mortalidade."),
            (Paragraph, "O BNP e o NT-proBNP sobem na IC e ajudam no diagnóstico de dispneia."),
        ]),
        page("Controle de constitucionalidade", &[
            (Paragraph, "No Brasil o controle é misto: difuso e concentrado."),
            (Paragraph, "O controle difuso pode ser feito por qualquer juiz ou tribunal, em um caso concreto, e o efeito em regra é inter partes. Nos tribunais, a declaração de inconstitucionalidade exige a maioria absoluta dos membros (cláusula de reserva de plenário, art. 97 da CF)."),
            (Paragraph, "O controle concentrado é feito pelo STF por meio de ADI, ADC, ADO e ADPF, com efeito erga omnes e vinculante. A ADPF é subsidiária: só cabe quando não houver outro meio eficaz."),
            (Paragraph, "Legitimados para a ADI (art. 103): Presidente da República, Mesas do Senado e da Câmara, Procurador-Geral da República, Conselho Federal da OAB, partido político com representação no Congresso, entre outros."),
        ]),
        page("Inibidores da ECA", &[(Paragraph, "Bloqueiam a conversão de angiotensina I em angiotensina II e reduzem a degradação de bradicinina (tosse, angioedema). Contraindicados na gestação.")]),
        page("Cell cycle and mitosis", &[
            (Paragraph, "The cell cycle is the sequence of events in which a cell duplicates its contents and divides in two. It is split into interphase (G1, S and G2) and the M phase, which includes mitosis and cytokinesis."),
            (H2, "Phases of mitosis"),
            (Numbered, "Prophase — chromatin condenses and the mitotic spindle starts to form."),
            (Numbered, "Metaphase — chromosomes line up on the metaphase plate, attached to the spindle by their kinetochores."),
            (Numbered, "Anaphase — sister chromatids separate and move to opposite poles."),
            (Numbered, "Telophase — the nuclear envelopes re-form; cytokinesis follows."),
            (H2, "Regulation and checkpoints"),
            (Paragraph, "The G1/S, G2/M and spindle checkpoints stop the cell from moving on with damaged DNA or badly attached chromosomes. Failures in these controls show up in cancer."),
        ]),
        page("Photosynthesis", &[
            (Paragraph, "Photosynthesis turns light energy into chemical energy stored in glucose. It happens in the chloroplasts and has two stages."),
            (H3, "Light-dependent reactions"),
            (Paragraph, "They take place in the thylakoid membranes. Water is split (photolysis), releasing oxygen, and the energy is stored as ATP and NADPH."),
            (H3, "Calvin cycle"),
            (Paragraph, "It takes place in the stroma. The enzyme RuBisCO fixes CO2 onto ribulose bisphosphate, and ATP and NADPH from the first stage are used to make G3P, which builds glucose."),
            (Paragraph, "Limiting factors: light intensity, CO2 concentration and temperature."),
        ]),
        page("Plasma membrane", &[(Paragraph, "The fluid mosaic model: a phospholipid bilayer with integral and peripheral proteins; cholesterol modulates fluidity.")]),
        page("The French Revolution", &[
            (Paragraph, "It began in 1789, during a fiscal crisis, when Louis XVI called the Estates-General. The Third Estate declared itself the National Assembly and took the Tennis Court Oath."),
            (Paragraph, "On 14 July 1789 the Bastille was stormed. In August the Assembly abolished feudal privileges and approved the Declaration of the Rights of Man and of the Citizen."),
            (Paragraph, "The monarchy was abolished in 1792 and Louis XVI was executed in 1793. The Reign of Terror (1793–1794) was led by Robespierre and the Committee of Public Safety."),
            (Paragraph, "It ended in 1799 with Napoleon Bonaparte's coup of 18 Brumaire."),
        ]),
    ]
}

#[test]
#[ignore]
fn eval_cards() {
    let model = std::env::var("DISCITE_EVAL_MODEL").expect("DISCITE_EVAL_MODEL");
    let out = std::env::var("DISCITE_EVAL_OUT").expect("DISCITE_EVAL_OUT");
    let gpu = std::env::var_os("DISCITE_EVAL_CPU").is_none();
    let mut lines = vec![];
    for (title, blocks) in corpus() {
        let source = prompt::source(&title, &blocks);
        let count = prompt::cap(10, &source);
        let req = prompt::Request { source: &source, existing: &[], count, style: Style::Auto, focus: None };
        let job = engine::generate(
            GenRequest {
                model: model.clone().into(),
                gpu,
                prompt: prompt::prompt(&req),
                grammar: prompt::grammar(Style::Auto, count),
                ids: source.ids.clone(),
                notes: source.text.clone(),
                max_tokens: count as usize * 220 + 200,
            },
            || {},
        );
        let mut cards = vec![];
        loop {
            match job.events.recv() {
                Ok(Event::Card(s)) => cards.push(s),
                Ok(Event::Finished { secs }) => {
                    eprintln!("{title}: {} cards in {secs:.1}s", cards.len());
                    break;
                }
                Ok(Event::Failed(e)) => panic!("{title}: {e}"),
                Ok(_) => {}
                Err(_) => panic!("{title}: engine stopped"),
            }
        }
        for c in &cards {
            let block = c.block.and_then(|id| blocks.iter().find(|b| b.id == id)).map(|b| b.plain()).unwrap_or_default();
            lines.push(serde_json::json!({
                "page": title,
                "kind": format!("{:?}", c.kind),
                "front": c.front,
                "back": c.back,
                "block": block,
                "quote": c.quote,
                "grounded": c.grounded,
                "missing": c.missing,
            }).to_string());
        }
    }
    std::fs::write(&out, lines.join("\n")).unwrap();
}
