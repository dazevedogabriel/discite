//! Log de diagnóstico: um arquivo por sessão em `<pasta de dados>/logs`.
//! O da sessão anterior fica guardado como `discite.previous.log`, para dar
//! para ver o que aconteceu mesmo quando o programa fechou sozinho. Entram os
//! passos da IA, os avisos e erros do llama.cpp, os erros mostrados na tela e
//! qualquer pânico. As Configurações mostram os dois arquivos para copiar.

use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

struct Log {
    dir: PathBuf,
    file: Option<std::fs::File>,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

pub fn dir() -> PathBuf {
    LOG.get().map(|l| l.lock().unwrap().dir.clone()).unwrap_or_else(|| crate::db::data_root().join("logs"))
}

pub fn current_path() -> PathBuf {
    dir().join("discite.log")
}

pub fn previous_path() -> PathBuf {
    dir().join("discite.previous.log")
}

/// Abre o log desta sessão (o anterior vira `.previous`) e instala o gancho de
/// pânico. Chamado uma vez, no começo do `main`.
pub fn init() {
    let dir = crate::db::data_root().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let cur = dir.join("discite.log");
    if cur.exists() {
        let _ = std::fs::rename(&cur, dir.join("discite.previous.log"));
    }
    let file = std::fs::File::create(&cur).ok();
    let _ = LOG.set(Mutex::new(Log { dir, file }));
    info(format!("Discite {} · {} {} · {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));
    info(format!("CPU: {}", cpu_features()));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |p| {
        let thread = std::thread::current().name().unwrap_or("?").to_owned();
        error(format!("panic in thread '{thread}': {p}\n{}", std::backtrace::Backtrace::force_capture()));
        default(p);
    }));
    llama_logs();
}

/// Recursos da CPU que o llama.cpp usa: um Mac Intel antigo sem AVX2, por
/// exemplo, fecharia o programa ao gerar cartões.
fn cpu_features() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        let f = [
            ("sse4.2", std::arch::is_x86_feature_detected!("sse4.2")),
            ("avx", std::arch::is_x86_feature_detected!("avx")),
            ("avx2", std::arch::is_x86_feature_detected!("avx2")),
            ("fma", std::arch::is_x86_feature_detected!("fma")),
            ("f16c", std::arch::is_x86_feature_detected!("f16c")),
            ("avx512f", std::arch::is_x86_feature_detected!("avx512f")),
        ];
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
        format!("{threads} threads · {}", f.iter().map(|(n, on)| format!("{n} {}", if *on { "yes" } else { "no" })).collect::<Vec<_>>().join(", "))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
        format!("{threads} threads · {}", std::env::consts::ARCH)
    }
}

fn write(level: &str, msg: &str) {
    let Some(log) = LOG.get() else { return };
    let mut log = log.lock().unwrap();
    if let Some(f) = log.file.as_mut() {
        let t = chrono::Local::now().format("%H:%M:%S%.3f");
        for (i, line) in msg.trim_end().lines().enumerate() {
            let _ = if i == 0 { writeln!(f, "{t} {level:5} {line}") } else { writeln!(f, "{:18} {line}", "") };
        }
        // Sem buffer: se o processo morrer logo depois, a linha já está no disco.
        let _ = f.flush();
    }
}

pub fn info(msg: impl Display) {
    write("INFO", &msg.to_string());
}

pub fn warn(msg: impl Display) {
    write("WARN", &msg.to_string());
}

pub fn error(msg: impl Display) {
    write("ERROR", &msg.to_string());
}

/// Lê um arquivo de log (só o fim, se for grande).
pub fn read(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    const MAX: usize = 400_000;
    let start = bytes.len().saturating_sub(MAX);
    Some(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

/// Avisos e erros do llama.cpp vão para este log (via `tracing`).
fn llama_logs() {
    struct Sink;
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(buf);
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                write("LLAMA", line.trim());
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let _ = tracing_subscriber::fmt().with_writer(|| Sink).with_max_level(tracing::Level::WARN).with_target(false).without_time().with_level(true).try_init();
    llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
}
