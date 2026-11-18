//! Log do agente: vai para o arquivo (logs/balanca.log), para o console quando
//! há um, e para a aba Log da janela, que guarda as últimas linhas na memória.
//!
//! Só as mensagens do próprio agente entram. As bibliotecas de janela e de
//! rede escrevem muito em DEBUG e esconderiam o que importa.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

const LINHAS_NA_MEMORIA: usize = 500;

#[derive(Debug, Clone)]
pub struct LinhaLog {
    pub hora: String,
    pub nivel: Level,
    pub texto: String,
}

struct Registro {
    linhas: Mutex<VecDeque<LinhaLog>>,
    arquivo: Mutex<Option<File>>,
}

static REGISTRO: OnceLock<Registro> = OnceLock::new();

impl Log for Registro {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.target().starts_with("balanca_agent") && metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let agora = chrono::Local::now();
        let nivel = match record.level() {
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
            Level::Info => "INFO ",
            _ => "DEBUG",
        };
        let texto = record.args().to_string();

        eprintln!("[{}] {nivel} {texto}", agora.format("%H:%M:%S"));

        if let Ok(mut arquivo) = self.arquivo.lock() {
            if let Some(f) = arquivo.as_mut() {
                let _ = writeln!(f, "[{}] {nivel} {texto}", agora.format("%Y-%m-%d %H:%M:%S"));
            }
        }

        if let Ok(mut linhas) = self.linhas.lock() {
            if linhas.len() >= LINHAS_NA_MEMORIA {
                linhas.pop_front();
            }
            linhas.push_back(LinhaLog { hora: agora.format("%H:%M:%S").to_string(), nivel: record.level(), texto });
        }
    }

    fn flush(&self) {
        if let Ok(mut arquivo) = self.arquivo.lock() {
            if let Some(f) = arquivo.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

/// Liga o log. Pode ser chamada mais de uma vez (nos testes); só a primeira
/// instala o logger.
pub fn iniciar(caminho: &Path, nivel: &str) {
    let arquivo = caminho
        .parent()
        .map(std::fs::create_dir_all)
        .and_then(|r| r.ok())
        .and_then(|_| OpenOptions::new().create(true).append(true).open(caminho).ok());

    let registro = REGISTRO.get_or_init(|| Registro { linhas: Mutex::new(VecDeque::new()), arquivo: Mutex::new(None) });
    if let Ok(mut atual) = registro.arquivo.lock() {
        *atual = arquivo;
    }
    let _ = log::set_logger(registro);
    definir_nivel(nivel);
}

pub fn definir_nivel(nivel: &str) {
    log::set_max_level(match nivel {
        "DEBUG" => LevelFilter::Debug,
        "WARNING" => LevelFilter::Warn,
        "ERROR" => LevelFilter::Error,
        _ => LevelFilter::Info,
    });
}

pub fn ultimas_linhas() -> Vec<LinhaLog> {
    REGISTRO
        .get()
        .and_then(|r| r.linhas.lock().ok().map(|l| l.iter().cloned().collect()))
        .unwrap_or_default()
}

pub fn limpar_memoria() {
    if let Some(r) = REGISTRO.get() {
        if let Ok(mut l) = r.linhas.lock() {
            l.clear();
        }
    }
}
