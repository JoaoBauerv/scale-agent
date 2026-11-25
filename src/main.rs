//! Scale Agent — agente de balança para desktop, em Rust.
//!
//!     scale-agent.exe               abre a janela
//!     scale-agent.exe --minimizado  abre minimizado (é o que o "Iniciar com o Windows" usa)
//!     scale-agent.exe --sem-janela  só a API, sem janela (servidor, testes)
//!     scale-agent.exe --aba=configuracoes   abre direto numa aba (configuracoes, integracao, log)
//!
//! Sobe a API local (padrão http://127.0.0.1:5000) com as mesmas rotas do
//! agente Python e a janela de acompanhamento e configuração.

// Sem console no Windows na versão final; em debug o console mostra o log.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod autostart;
mod balanca;
mod config;
mod erro;
mod gui;
mod registro;
mod servico;
mod servidor;

use std::sync::{Arc, Mutex};

use log::{info, warn};

use crate::servico::ServicoPesagem;
use crate::servidor::ServidorApi;

fn main() -> eframe::Result<()> {
    let argumentos: Vec<String> = std::env::args().skip(1).collect();
    let minimizado = argumentos.iter().any(|a| a == "--minimizado");
    let sem_janela = argumentos.iter().any(|a| a == "--sem-janela");
    let aba = argumentos.iter().find_map(|a| a.strip_prefix("--aba="));

    let (mut cfg, aviso) = config::carregar();
    registro::iniciar(&config::caminho_log(), &cfg.log_nivel);

    info!("{}", "=".repeat(58));
    info!("Scale Agent {} iniciando", env!("CARGO_PKG_VERSION"));
    info!("Modo: {} | Unidade: {}", cfg.modo_balanca.como_str(), cfg.unidade);
    if cfg.modo_balanca == config::Modo::Real {
        info!("Porta: {} | Baudrate: {}", cfg.porta_com, cfg.baudrate);
    }
    info!("Configuração: {}", config::caminho_config().display());
    info!("{}", "=".repeat(58));
    if let Some(a) = &aviso {
        warn!("{a}");
    }
    for problema in cfg.validar() {
        warn!("Configuração: {problema}");
    }

    // A verdade sobre "iniciar com o Windows" é o registro, não o arquivo.
    cfg.iniciar_com_windows = autostart::ativo();

    let servico = ServicoPesagem::novo(cfg.clone());
    servico.iniciar();

    let servidor = Arc::new(Mutex::new(None));
    let erro_servidor = match ServidorApi::iniciar(&cfg, Arc::clone(&servico)) {
        Ok(s) => {
            *servidor.lock().unwrap() = Some(s);
            None
        }
        Err(e) => {
            warn!("{e}");
            Some(e)
        }
    };

    if sem_janela {
        if erro_servidor.is_some() {
            std::process::exit(1);
        }
        loop {
            std::thread::park();
        }
    }

    let opcoes = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Scale Agent - Agente de Balança")
            .with_inner_size([980.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("ícone embutido inválido"),
            ),
        ..Default::default()
    };

    let app = gui::App::new(Arc::clone(&servico), Arc::clone(&servidor), erro_servidor, cfg, minimizado, aviso, aba);
    let resultado = eframe::run_native("Scale Agent", opcoes, Box::new(move |_cc| Ok(Box::new(app))));

    if let Some(mut s) = servidor.lock().unwrap().take() {
        s.parar();
    }
    servico.parar();
    info!("Agente encerrado");
    log::logger().flush();
    resultado
}
