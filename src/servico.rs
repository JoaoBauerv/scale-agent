//! Serviço de pesagem: orquestra a balança ativa, a leitura periódica e a
//! estabilidade.
//!
//! É a única parte do agente que sabe se a balança é real ou simulada. A API e
//! a janela conversam só com este serviço.
//!
//! O estado publicado fica num lugar separado da balança: ler o peso (API,
//! janela) nunca espera a porta serial, que pode levar até o timeout numa
//! leitura.

use std::collections::VecDeque;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{debug, error, info};
use serde_json::Value;

use crate::balanca::serial::BalancaSerial;
use crate::balanca::simulada::BalancaSimulada;
use crate::balanca::{Balanca, BalancaIndisponivel};
use crate::config::{Config, Modo};
use crate::erro::{Codigo, ErroBalanca};

#[derive(Debug, Clone)]
pub struct EstadoPesagem {
    pub peso: Option<f64>,
    pub unidade: String,
    pub estavel: bool,
    /// "simulado" ou "real", como no agente Python.
    pub modo: String,
    pub conectada: bool,
    pub erro: Option<String>,
    pub codigo_erro: Option<String>,
    pub atualizado_em: f64,
    pub conexao: Value,
    pub ultimo_frame: Option<String>,
}

impl EstadoPesagem {
    fn inicial(cfg: &Config) -> Self {
        Self {
            peso: None,
            unidade: cfg.unidade.clone(),
            estavel: false,
            modo: modo_publicado(cfg.modo_balanca).into(),
            conectada: false,
            erro: None,
            codigo_erro: None,
            atualizado_em: agora_unix(),
            conexao: Value::Null,
            ultimo_frame: None,
        }
    }
}

fn modo_publicado(modo: Modo) -> &'static str {
    match modo {
        Modo::Simulada => "simulado",
        Modo::Real => "real",
    }
}

fn agora_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

struct Interno {
    config: Config,
    balanca: Box<dyn Balanca>,
    historico: VecDeque<(Instant, f64)>,
    ultimo_codigo_logado: Option<Codigo>,
}

pub struct ServicoPesagem {
    interno: Mutex<Interno>,
    estado: RwLock<EstadoPesagem>,
    laco: Mutex<Option<(Sender<()>, JoinHandle<()>)>>,
}

impl ServicoPesagem {
    pub fn novo(config: Config) -> Arc<Self> {
        let balanca = criar_balanca(&config, config.modo_balanca);
        let estado = EstadoPesagem { conexao: balanca.descricao(), ..EstadoPesagem::inicial(&config) };
        info!("Modo inicial: {}", config.modo_balanca.como_str());
        Arc::new(Self {
            interno: Mutex::new(Interno { config, balanca, historico: VecDeque::with_capacity(256), ultimo_codigo_logado: None }),
            estado: RwLock::new(estado),
            laco: Mutex::new(None),
        })
    }

    // ------------------------------------------------------------------ ciclo
    pub fn iniciar(self: &Arc<Self>) {
        let mut laco = self.laco.lock().unwrap();
        if laco.is_some() {
            return;
        }
        let (parar, sinal) = mpsc::channel::<()>();
        let servico = Arc::clone(self);
        let thread = std::thread::Builder::new()
            .name("leitura-balanca".into())
            .spawn(move || loop {
                servico.ciclo();
                let intervalo = servico.intervalo();
                match sinal.recv_timeout(intervalo) {
                    Err(RecvTimeoutError::Timeout) => continue,
                    _ => break,
                }
            })
            .expect("não foi possível criar a thread de leitura");
        *laco = Some((parar, thread));
        info!("Serviço de pesagem iniciado (intervalo de {:.2}s)", self.intervalo().as_secs_f64());
    }

    pub fn parar(&self) {
        if let Some((parar, thread)) = self.laco.lock().unwrap().take() {
            let _ = parar.send(());
            let _ = thread.join();
        }
        if let Ok(mut interno) = self.interno.lock() {
            interno.balanca.desconectar();
        }
        info!("Serviço de pesagem encerrado");
    }

    fn intervalo(&self) -> Duration {
        let segundos = self.interno.lock().map(|i| i.config.intervalo_atualizacao).unwrap_or(0.5);
        Duration::from_secs_f64(segundos.max(0.05))
    }

    /// Uma leitura. Chamada pela thread e logo depois de cada comando, para o
    /// estado publicado não ficar com o peso de antes até o próximo ciclo.
    pub fn ciclo(&self) {
        let mut interno = match self.interno.lock() {
            Ok(i) => i,
            Err(envenenado) => envenenado.into_inner(),
        };

        if !interno.balanca.esta_conectada() {
            if let Err(e) = interno.balanca.conectar() {
                self.registrar_falha(&mut interno, e);
                return;
            }
            let descricao = descricao_curta(&interno.balanca.descricao());
            info!("Balança conectada ({descricao})");
            interno.ultimo_codigo_logado = None;
        }

        let leitura = match interno.balanca.ler_peso() {
            Ok(l) => l,
            Err(e) => {
                self.registrar_falha(&mut interno, e);
                return;
            }
        };

        let agora = Instant::now();
        if interno.historico.len() >= 256 {
            interno.historico.pop_front();
        }
        interno.historico.push_back((agora, leitura.peso));

        let estavel = leitura.estavel.unwrap_or_else(|| calcular_estabilidade(&interno.historico, &interno.config, agora));
        let unidade = if leitura.unidade.is_empty() { interno.config.unidade.clone() } else { leitura.unidade };

        let novo = EstadoPesagem {
            peso: Some(leitura.peso),
            unidade: unidade.clone(),
            estavel,
            modo: interno.balanca.modo().into(),
            conectada: true,
            erro: None,
            codigo_erro: None,
            atualizado_em: agora_unix(),
            conexao: interno.balanca.descricao(),
            ultimo_frame: interno.balanca.ultimo_frame(),
        };
        interno.ultimo_codigo_logado = None;

        let mut estado = self.estado.write().unwrap();
        if estado.peso != novo.peso {
            debug!("Peso recebido: {:.2} {unidade}", leitura.peso);
        }
        if estavel && !estado.estavel {
            info!("Peso estável: {:.2} {unidade}", leitura.peso);
        }
        *estado = novo;
    }

    fn registrar_falha(&self, interno: &mut Interno, e: ErroBalanca) {
        interno.historico.clear();

        // A mesma falha a cada ciclo não enche o log: só a primeira entra.
        if interno.ultimo_codigo_logado != Some(e.codigo) {
            error!("{}: {}", e.codigo.como_str(), e.mensagem);
            interno.ultimo_codigo_logado = Some(e.codigo);
        }

        *self.estado.write().unwrap() = EstadoPesagem {
            peso: None,
            unidade: interno.config.unidade.clone(),
            estavel: false,
            modo: interno.balanca.modo().into(),
            conectada: false,
            erro: Some(e.mensagem),
            codigo_erro: Some(e.codigo.como_str().into()),
            atualizado_em: agora_unix(),
            conexao: interno.balanca.descricao(),
            ultimo_frame: interno.balanca.ultimo_frame(),
        };
    }

    // ------------------------------------------------------------------ estado
    pub fn estado(&self) -> EstadoPesagem {
        self.estado.read().unwrap().clone()
    }

    pub fn config(&self) -> Config {
        self.interno.lock().unwrap().config.clone()
    }

    // ---------------------------------------------------------------- comandos
    /// Troca entre simulada e real sem reiniciar. Não grava na configuração:
    /// ao reabrir, vale o modo das Configurações, como no agente Python.
    pub fn definir_modo(&self, modo: &str) -> Result<EstadoPesagem, String> {
        let desejado = match modo.trim().to_lowercase().as_str() {
            "simulada" | "simulado" => Modo::Simulada,
            "real" => Modo::Real,
            _ => return Err("Modo inválido. Use 'simulada' ou 'real'.".into()),
        };
        {
            let mut interno = self.interno.lock().unwrap();
            if interno.balanca.modo() == modo_publicado(desejado) {
                return Ok(self.estado());
            }
            trocar_balanca(&mut interno, desejado);
            info!("Modo alterado para: {}", desejado.como_str());
        }
        self.ciclo();
        Ok(self.estado())
    }

    /// Aplica uma configuração nova (botão Salvar da janela): recria a balança
    /// no modo configurado.
    pub fn reconfigurar(&self, config: Config) {
        {
            let mut interno = self.interno.lock().unwrap();
            let modo = config.modo_balanca;
            interno.config = config;
            trocar_balanca(&mut interno, modo);
        }
        info!("Configuração aplicada");
        self.ciclo();
    }

    pub fn zerar(&self) -> Result<(), ErroBalanca> {
        {
            let mut interno = self.interno.lock().unwrap();
            interno.balanca.zerar()?;
            interno.historico.clear();
        }
        info!("Comando ZERO enviado à balança");
        self.ciclo();
        Ok(())
    }

    pub fn tarar(&self) -> Result<(), ErroBalanca> {
        {
            let mut interno = self.interno.lock().unwrap();
            interno.balanca.tarar()?;
            interno.historico.clear();
        }
        info!("Comando TARA enviado à balança");
        self.ciclo();
        Ok(())
    }

    /// Só no modo simulado (botão "Gerar Peso").
    pub fn gerar_peso(&self, valor: Option<f64>) -> Result<f64, String> {
        let (alvo, unidade) = {
            let mut interno = self.interno.lock().unwrap();
            let unidade = interno.config.unidade.clone();
            let simulada = interno
                .balanca
                .como_simulada()
                .ok_or_else(|| "Gerar peso só está disponível no modo simulado".to_string())?;
            let alvo = simulada.gerar_peso(valor);
            interno.historico.clear();
            (alvo, unidade)
        };
        info!("Simulação: novo peso de {alvo:.2} {unidade}");
        self.ciclo();
        Ok(alvo)
    }

    pub fn registrar_captura(&self, peso: f64, origem: &str) {
        let unidade = self.estado().unidade;
        info!("Peso capturado: {peso:.2} {unidade} (origem: {origem})");
    }
}

fn trocar_balanca(interno: &mut Interno, modo: Modo) {
    interno.balanca.desconectar();
    interno.historico.clear();
    interno.ultimo_codigo_logado = None;
    interno.balanca = criar_balanca(&interno.config, modo);
}

fn criar_balanca(cfg: &Config, modo: Modo) -> Box<dyn Balanca> {
    match modo {
        Modo::Simulada => Box::new(BalancaSimulada::nova(
            cfg.sim_peso_minimo,
            cfg.sim_peso_maximo,
            cfg.sim_peso_inicial,
            cfg.sim_resolucao,
            &cfg.unidade,
            cfg.casas_decimais,
        )),
        Modo::Real => match cfg.protocolo() {
            Ok(protocolo) => Box::new(BalancaSerial::nova(cfg.parametros_serial(), protocolo)),
            Err(motivo) => Box::new(BalancaIndisponivel { motivo }),
        },
    }
}

/// O peso é estável quando ficou dentro da tolerância durante toda a janela
/// de tempo — não basta duas leituras seguidas iguais.
fn calcular_estabilidade(historico: &VecDeque<(Instant, f64)>, cfg: &Config, agora: Instant) -> bool {
    if cfg.tempo_estabilidade <= 0.0 {
        return true;
    }
    let janela = Duration::from_secs_f64(cfg.tempo_estabilidade);
    let Some(limite) = agora.checked_sub(janela) else {
        return false;
    };
    match historico.front() {
        Some((primeiro, _)) if *primeiro <= limite => {}
        _ => return false,
    }
    let amostras: Vec<f64> = historico.iter().filter(|(t, _)| *t >= limite).map(|(_, p)| *p).collect();
    if amostras.len() < 2 {
        return false;
    }
    let max = amostras.iter().cloned().fold(f64::MIN, f64::max);
    let min = amostras.iter().cloned().fold(f64::MAX, f64::min);
    max - min <= cfg.tolerancia_estabilidade + 1e-9
}

fn descricao_curta(descricao: &Value) -> String {
    if descricao["modo"] == "real" {
        format!("porta {} @ {} baud", descricao["porta"].as_str().unwrap_or("?"), descricao["baudrate"])
    } else {
        "modo simulado".into()
    }
}

#[cfg(test)]
pub mod testes {
    use super::*;

    pub fn config_rapida() -> Config {
        Config {
            intervalo_atualizacao: 0.05,
            tempo_estabilidade: 0.2,
            tolerancia_estabilidade: 0.05,
            porta_com: "COM_TESTE_INEXISTENTE".into(),
            timeout: 0.2,
            ..Config::default()
        }
    }

    fn esperar(limite: Duration, mut condicao: impl FnMut() -> bool) -> bool {
        let fim = Instant::now() + limite;
        while Instant::now() < fim {
            if condicao() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn detecta_peso_estavel() {
        let servico = ServicoPesagem::novo(config_rapida());
        servico.iniciar();
        servico.gerar_peso(Some(40.0)).unwrap();
        let ok = esperar(Duration::from_secs(5), || {
            let e = servico.estado();
            e.estavel && (e.peso.unwrap_or(0.0) - 40.0).abs() < 0.05
        });
        servico.parar();
        assert!(ok, "o peso nunca foi considerado estável");
    }

    #[test]
    fn marca_instavel_logo_apos_mudanca_de_peso() {
        let servico = ServicoPesagem::novo(config_rapida());
        servico.iniciar();
        assert!(esperar(Duration::from_secs(5), || servico.estado().estavel));
        servico.gerar_peso(Some(80.0)).unwrap();
        let estado = servico.estado();
        servico.parar();
        assert!(!estado.estavel);
        assert_eq!(estado.peso, Some(80.0));
    }

    #[test]
    fn troca_de_modo() {
        let servico = ServicoPesagem::novo(config_rapida());
        servico.iniciar();
        assert_eq!(servico.estado().modo, "simulado");

        servico.definir_modo("real").unwrap();
        let estado = servico.estado();
        assert_eq!(estado.modo, "real");
        assert!(!estado.conectada);
        assert_eq!(estado.codigo_erro.as_deref(), Some("PORTA_INDISPONIVEL"));
        assert!(servico.gerar_peso(None).is_err(), "gerar peso não vale no modo real");

        servico.definir_modo("simulada").unwrap();
        assert_eq!(servico.estado().modo, "simulado");
        assert!(servico.definir_modo("xpto").is_err());
        servico.parar();
    }

    #[test]
    fn reconfigurar_aplica_o_modo_e_a_unidade() {
        let servico = ServicoPesagem::novo(config_rapida());
        servico.reconfigurar(Config { unidade: "g".into(), sim_peso_maximo: 5000.0, ..config_rapida() });
        servico.gerar_peso(Some(1234.0)).unwrap();
        let estado = servico.estado();
        assert_eq!(estado.unidade, "g");
        assert_eq!(estado.peso, Some(1234.0));
    }
}
