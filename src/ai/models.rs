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
    models_dir().join(format!("{}.part", m.file))
}

/// Quanto de um download interrompido já está no disco.
pub fn partial_bytes(m: &ModelSpec) -> u64 {
    std::fs::metadata(part_path(m)).map(|md| md.len()).unwrap_or(0)
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
    pub model: &'static str,
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
        let progress = Arc::new(Mutex::new(Progress { model: m.id, done: partial_bytes(m), total: m.size, phase: Phase::Downloading, speed: 0.0 }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c) = (progress.clone(), cancel.clone());
        std::thread::spawn(move || {
            let result = fetch(m, &p, &c, &repaint);
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

fn fetch(m: &ModelSpec, progress: &Mutex<Progress>, cancel: &AtomicBool, repaint: &dyn Fn()) -> Result<()> {
    std::fs::create_dir_all(models_dir())?;
    let part = part_path(m);
    let mut have = partial_bytes(m);
    if have < m.size {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .user_agent(concat!("Discite/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let mut req = agent.get(m.url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let resp = req.call()?;
        // 206 continua de onde parou; 200 manda o arquivo inteiro de novo.
        let resume = resp.status().as_u16() == 206;
        if !resume {
            have = 0;
        }
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
        if have < m.size {
            bail!("incomplete download ({have} of {} bytes)", m.size);
        }
    }
    progress.lock().unwrap().phase = Phase::Verifying;
    repaint();
    let sum = sha256_file(&part, cancel)?;
    if !sum.eq_ignore_ascii_case(m.sha256) {
        // Arquivo corrompido: começa do zero na próxima tentativa.
        let _ = std::fs::remove_file(&part);
        bail!("checksum mismatch");
    }
    std::fs::rename(&part, path_of(m))?;
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
