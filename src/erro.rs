//! Erros da balança. O `codigo` é o que a API devolve no JSON, igual ao agente
//! Python, então o sistema web não precisa saber qual dos dois está rodando.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codigo {
    BalancaOffline,
    PortaIndisponivel,
    Timeout,
    DadosInvalidos,
    ComandoNaoSuportado,
    /// Genérico, mantido para o mesmo conjunto de códigos do agente Python.
    #[allow(dead_code)]
    ErroBalanca,
}

impl Codigo {
    pub fn como_str(self) -> &'static str {
        match self {
            Codigo::BalancaOffline => "BALANCA_OFFLINE",
            Codigo::PortaIndisponivel => "PORTA_INDISPONIVEL",
            Codigo::Timeout => "TIMEOUT",
            Codigo::DadosInvalidos => "DADOS_INVALIDOS",
            Codigo::ComandoNaoSuportado => "COMANDO_NAO_SUPORTADO",
            Codigo::ErroBalanca => "ERRO_BALANCA",
        }
    }

    pub fn mensagem_padrao(self) -> &'static str {
        match self {
            Codigo::BalancaOffline => "Balança não conectada",
            Codigo::PortaIndisponivel => "Porta serial inexistente ou já em uso por outro programa",
            Codigo::Timeout => "A balança não respondeu dentro do tempo limite",
            Codigo::DadosInvalidos => "Não foi possível interpretar os dados enviados pela balança",
            Codigo::ComandoNaoSuportado => "A balança configurada não suporta este comando",
            Codigo::ErroBalanca => "Erro na comunicação com a balança",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErroBalanca {
    pub codigo: Codigo,
    pub mensagem: String,
}

impl ErroBalanca {
    pub fn novo(codigo: Codigo, mensagem: impl Into<String>) -> Self {
        Self { codigo, mensagem: mensagem.into() }
    }

    pub fn padrao(codigo: Codigo) -> Self {
        Self::novo(codigo, codigo.mensagem_padrao())
    }
}

impl fmt::Display for ErroBalanca {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.mensagem)
    }
}

impl std::error::Error for ErroBalanca {}
