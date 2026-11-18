//! Contrato comum às balanças (física ou simulada).
//!
//! O resto do agente conversa só com `Balanca`, então trocar o modelo da
//! balança física não exige mudança na API nem no sistema web.

pub mod serial;
pub mod simulada;

use crate::erro::{Codigo, ErroBalanca};

/// Uma leitura vinda da balança.
///
/// `estavel` é `None` quando a balança não informa estabilidade no protocolo:
/// nesse caso quem decide é o serviço de pesagem, comparando leituras seguidas.
#[derive(Debug, Clone, PartialEq)]
pub struct LeituraPeso {
    pub peso: f64,
    pub unidade: String,
    pub estavel: Option<bool>,
}

pub trait Balanca: Send {
    /// "simulado" ou "real" — o mesmo texto que o agente Python devolve.
    fn modo(&self) -> &'static str;
    fn conectar(&mut self) -> Result<(), ErroBalanca>;
    fn desconectar(&mut self);
    fn esta_conectada(&self) -> bool;
    fn ler_peso(&mut self) -> Result<LeituraPeso, ErroBalanca>;

    fn zerar(&mut self) -> Result<(), ErroBalanca> {
        Err(ErroBalanca::padrao(Codigo::ComandoNaoSuportado))
    }

    fn tarar(&mut self) -> Result<(), ErroBalanca> {
        Err(ErroBalanca::padrao(Codigo::ComandoNaoSuportado))
    }

    /// Informações da conexão, mostradas na janela e em /status.
    fn descricao(&self) -> serde_json::Value;

    /// Último frame cru recebido (só a balança física tem).
    fn ultimo_frame(&self) -> Option<String> {
        None
    }

    fn como_simulada(&mut self) -> Option<&mut simulada::BalancaSimulada> {
        None
    }
}

/// Arredonda para `casas` decimais, eliminando o ruído de ponto flutuante
/// (0.1 * 3 = 0.30000000000000004).
pub fn arredondar(valor: f64, casas: u32) -> f64 {
    let fator = 10f64.powi(casas as i32);
    (valor * fator).round() / fator
}

/// Balança que não pode ser criada (ex.: regex inválida num config.toml
/// editado à mão). Em vez de derrubar o agente, responde com o motivo.
pub struct BalancaIndisponivel {
    pub motivo: String,
}

impl Balanca for BalancaIndisponivel {
    fn modo(&self) -> &'static str {
        "real"
    }
    fn conectar(&mut self) -> Result<(), ErroBalanca> {
        Err(ErroBalanca::novo(Codigo::DadosInvalidos, self.motivo.clone()))
    }
    fn desconectar(&mut self) {}
    fn esta_conectada(&self) -> bool {
        false
    }
    fn ler_peso(&mut self) -> Result<LeituraPeso, ErroBalanca> {
        Err(ErroBalanca::novo(Codigo::DadosInvalidos, self.motivo.clone()))
    }
    fn descricao(&self) -> serde_json::Value {
        serde_json::json!({ "modo": "real", "erro": self.motivo })
    }
}
