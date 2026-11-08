//! Modelos oferecidos para a IA local e o download deles (retomável, com
//! conferência do SHA-256) para `<pasta de dados>/models`.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// Chave de tradução da descrição curta.
    pub blurb: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    /// Memória livre recomendada (GB).
    pub ram_gb: u32,
}

/// Qwen3 (Apache 2.0), quantizados em Q4_K_M pela Unsloth. O 4B Instruct
/// responde direto, sem a etapa de raciocínio, e foi o que escreveu os
/// melhores cartões nos testes (lacunas no lugar certo, bloco de origem
/// certo); o Qwen3.5 do mesmo tamanho esquecia as lacunas. Os outros dois
/// têm raciocínio, que o prompt desliga.
pub const CATALOG: [ModelSpec; 3] = [
    ModelSpec {
        id: "qwen3-1.7b",
        name: "Qwen3 1.7B",
        blurb: "ai.model_small",
        file: "Qwen3-1.7B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        sha256: "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
        size: 1_107_409_472,
        ram_gb: 4,
    },
    ModelSpec {
        id: "qwen3-4b-instruct",
        name: "Qwen3 4B Instruct",
        blurb: "ai.model_medium",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        size: 2_497_281_120,
        ram_gb: 6,
    },
    ModelSpec {
        id: "qwen3-8b",
        name: "Qwen3 8B",
        blurb: "ai.model_large",
        file: "Qwen3-8B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
        sha256: "120307ba529eb2439d6c430d94104dabd578497bc7bfe7e322b5d9933b449bd4",
        size: 5_027_784_512,
        ram_gb: 10,
    },
];

pub const DEFAULT_MODEL: &str = "qwen3-4b-instruct";

pub fn spec(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn models_dir() -> PathBuf {
    crate::db::data_root().join("models")
}

pub fn path_of(m: &ModelSpec) -> PathBuf {
    models_dir().join(m.file)
}

pub fn is_installed(m: &ModelSpec) -> bool {
    path_of(m).is_file()
}

pub fn remove(m: &ModelSpec) -> Result<()> {
    for p in [path_of(m), part_path(m)] {
        if p.exists() {
            std::fs::remove_file(&p).with_context(|| p.display().to_string())?;
        }
    }
    Ok(())
}

fn part_path(m: &ModelSpec) -> PathBuf {
    part_of(m.file)
}

fn part_of(file: &str) -> PathBuf {
    models_dir().join(format!("{file}.part"))
}

/// Quanto de um download interrompido já está no disco.
pub fn partial_bytes(m: &ModelSpec) -> u64 {
    std::fs::metadata(part_path(m)).map(|md| md.len()).unwrap_or(0)
}

/// Arquivos .gguf na pasta dos modelos que não são da lista (modo avançado).
pub fn custom_models() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(models_dir())
        .map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<PathBuf>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|p: &PathBuf| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gguf")))
        .filter(|p| !CATALOG.iter().any(|m| p.file_name().is_some_and(|n| n == m.file)))
        .collect();
    v.sort();
    v
}

/// Nome do arquivo de um link de download de .gguf (aceita o link "blob" do
/// Hugging Face, que é a página, trocando por "resolve", que é o arquivo).
pub fn gguf_link(url: &str) -> Option<(String, String)> {
    let url = url.trim().replace("/blob/", "/resolve/");
    let name = url.split(['?', '#']).next()?.rsplit('/').next()?.to_owned();
    (url.starts_with("http") && name.to_ascii_lowercase().ends_with(".gguf") && name.len() > 5).then_some((url, name))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Downloading,
    Verifying,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct Progress {
    /// Id do modelo da lista, ou o nome do arquivo num download por link.
    pub model: String,
    pub done: u64,
    pub total: u64,
    pub phase: Phase,
    /// Bytes por segundo (média dos últimos segundos).
    pub speed: f64,
}

/// Download rodando numa thread; a interface lê o progresso a cada quadro.
pub struct Download {
    pub progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
}

impl Download {
    pub fn start(m: &'static ModelSpec, repaint: impl Fn() + Send + 'static) -> Download {
        Download::begin(m.id.to_owned(), Target { url: m.url.to_owned(), file: m.file.to_owned(), size: Some(m.size), sha256: Some(m.sha256) }, repaint)
    }

    /// Download de um .gguf qualquer por link (modo avançado): sem SHA-256
    /// conhecido, confere só que o arquivo começa como um GGUF.
    pub fn start_url(url: String, file: String, repaint: impl Fn() + Send + 'static) -> Download {
        Download::begin(file.clone(), Target { url, file, size: None, sha256: None }, repaint)
    }

    fn begin(id: String, t: Target, repaint: impl Fn() + Send + 'static) -> Download {
        let have = std::fs::metadata(part_of(&t.file)).map(|md| md.len()).unwrap_or(0);
        let progress = Arc::new(Mutex::new(Progress { model: id, done: have, total: t.size.unwrap_or(0), phase: Phase::Downloading, speed: 0.0 }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c) = (progress.clone(), cancel.clone());
        std::thread::spawn(move || {
            let result = fetch(&t, &p, &c, &repaint);
            let phase = match result {
                Ok(()) => Phase::Done,
                Err(_) if c.load(Ordering::Relaxed) => Phase::Cancelled,
                Err(e) => Phase::Failed(format!("{e:#}")),
            };
            p.lock().unwrap().phase = phase;
            repaint();
        });
        Download { progress, cancel }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }
}

struct Target {
    url: String,
    file: String,
    size: Option<u64>,
    sha256: Option<&'static str>,
}

fn fetch(m: &Target, progress: &Mutex<Progress>, cancel: &AtomicBool, repaint: &dyn Fn()) -> Result<()> {
    std::fs::create_dir_all(models_dir())?;
    let part = part_of(&m.file);
    let mut have = std::fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
    if m.size.is_none_or(|s| have < s) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .user_agent(concat!("Discite/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let mut req = agent.get(&m.url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let resp = req.call()?;
        // 206 continua de onde parou; 200 manda o arquivo inteiro de novo.
        let resume = resp.status().as_u16() == 206;
        if !resume {
            have = 0;
        }
        // Tamanho total: o da lista ou o que o servidor informa.
        let total = m.size.unwrap_or_else(|| resp.body().content_length().map(|n| n + have).unwrap_or(0));
        progress.lock().unwrap().total = total;
        let mut file = std::fs::OpenOptions::new().create(true).write(true).append(resume).truncate(!resume).open(&part)?;
        let mut reader = resp.into_body().into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut window = (std::time::Instant::now(), have);
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            have += n as u64;
            let elapsed = window.0.elapsed().as_secs_f64();
            let mut p = progress.lock().unwrap();
            p.done = have;
            if elapsed >= 1.0 {
                p.speed = (have - window.1) as f64 / elapsed;
                window = (std::time::Instant::now(), have);
                drop(p);
                repaint();
            }
        }
        file.flush()?;
        if total > 0 && have < total {
            bail!("incomplete download ({have} of {total} bytes)");
        }
    }
    progress.lock().unwrap().phase = Phase::Verifying;
    repaint();
    match m.sha256 {
        Some(expected) => {
            let sum = sha256_file(&part, cancel)?;
            if !sum.eq_ignore_ascii_case(expected) {
                // Arquivo corrompido: começa do zero na próxima tentativa.
                let _ = std::fs::remove_file(&part);
                bail!("checksum mismatch");
            }
        }
        None => {
            // Link errado costuma baixar uma página HTML: o GGUF começa com "GGUF".
            let mut magic = [0u8; 4];
            std::fs::File::open(&part)?.read_exact(&mut magic)?;
            if &magic != b"GGUF" {
                let _ = std::fs::remove_file(&part);
                bail!("not a GGUF model file");
            }
        }
    }
    std::fs::rename(&part, models_dir().join(&m.file))?;
    Ok(())
}

fn sha256_file(path: &Path, cancel: &AtomicBool) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn gguf_links() {
        let (url, name) = super::gguf_link("https://huggingface.co/unsloth/Qwen3-14B-GGUF/blob/main/Qwen3-14B-Q4_K_M.gguf?download=true").unwrap();
        assert_eq!(url, "https://huggingface.co/unsloth/Qwen3-14B-GGUF/resolve/main/Qwen3-14B-Q4_K_M.gguf?download=true");
        assert_eq!(name, "Qwen3-14B-Q4_K_M.gguf");
        assert!(super::gguf_link("https://huggingface.co/unsloth/Qwen3-14B-GGUF").is_none());
    }
}
