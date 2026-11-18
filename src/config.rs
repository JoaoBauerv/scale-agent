//! Configuração do agente.
//!
//! São os mesmos parâmetros do `.env` do agente Python, com os mesmos nomes em
//! minúsculas, gravados em `config.toml`. Quem edita é a tela Configurações da
//! janela; o arquivo pode ser editado à mão, e o que estiver inválido nele cai
//! no padrão com aviso no log, em vez de impedir o agente de abrir.
//!
//! Onde fica: se existir um `config.toml` ao lado do executável, ele é usado
//! (modo portátil, útil num pendrive). Senão, %APPDATA%\BalancaAgent\.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::balanca::serial::{ParametrosSerial, Protocolo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modo {
    Simulada,
    Real,
}

impl Modo {
    pub fn como_str(self) -> &'static str {
        match self {
            Modo::Simulada => "simulada",
            Modo::Real => "real",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // Operação
    pub modo_balanca: Modo,
    pub unidade: String,

    // API local
    pub host: String,
    pub porta_api: u16,
    /// Separadas por vírgula ou por linha. localhost e 127.0.0.1 são sempre
    /// liberados; "*" libera qualquer origem.
    pub origens_permitidas: String,
    pub permitir_rede: bool,
    pub api_token: String,

    // Leitura / estabilidade
    pub intervalo_atualizacao: f64,
    pub tolerancia_estabilidade: f64,
    pub tempo_estabilidade: f64,
    pub casas_decimais: u32,

    // Simulada
    pub sim_peso_minimo: f64,
    pub sim_peso_maximo: f64,
    pub sim_peso_inicial: f64,
    pub sim_resolucao: f64,

    // Serial
    pub porta_com: String,
    pub baudrate: u32,
    pub bytesize: u8,
    pub paridade: String,
    pub stopbits: u8,
    pub timeout: f64,

    // Protocolo da balança física. Os comandos e o terminador aceitam
    // escapes: \r \n \t \0 \\ e \xHH.
    pub comando_peso: String,
    pub terminador: String,
    pub protocolo_regex: String,
    pub protocolo_regex_estavel: String,
    pub fator_divisao: f64,
    pub comando_zero: String,
    pub comando_tara: String,

    // Log
    pub log_nivel: String,

    // Windows
    pub iniciar_com_windows: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            modo_balanca: Modo::Simulada,
            unidade: "kg".into(),
            host: "127.0.0.1".into(),
            porta_api: 5000,
            origens_permitidas: "http://localhost,http://127.0.0.1".into(),
            permitir_rede: false,
            api_token: String::new(),
            intervalo_atualizacao: 0.5,
            tolerancia_estabilidade: 0.05,
            tempo_estabilidade: 1.5,
            casas_decimais: 2,
            sim_peso_minimo: 0.0,
            sim_peso_maximo: 200.0,
            sim_peso_inicial: 0.0,
            sim_resolucao: 0.10,
            porta_com: "COM3".into(),
            baudrate: 9600,
            bytesize: 8,
            paridade: "N".into(),
            stopbits: 1,
            timeout: 1.0,
            comando_peso: String::new(),
            terminador: "\\r\\n".into(),
            protocolo_regex: r"([-+]?\d+[.,]?\d*)".into(),
            protocolo_regex_estavel: String::new(),
            fator_divisao: 1.0,
            comando_zero: String::new(),
            comando_tara: String::new(),
            log_nivel: "INFO".into(),
            iniciar_com_windows: false,
        }
    }
}

pub const NIVEIS_LOG: [&str; 4] = ["DEBUG", "INFO", "WARNING", "ERROR"];
pub const PARIDADES: [(&str, &str); 3] = [("N", "Nenhuma (N)"), ("E", "Par (E)"), ("O", "Ímpar (O)")];
pub const BAUDRATES: [u32; 9] = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400];

impl Config {
    /// Origens liberadas no CORS, além de localhost/127.0.0.1.
    pub fn origens(&self) -> Vec<String> {
        self.origens_permitidas
            .split([',', '\n', ';'])
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Endereço que o sistema web deve usar para falar com este agente.
    pub fn url_para_o_sistema(&self) -> String {
        let host = match self.host.as_str() {
            "0.0.0.0" | "::" | "" => "127.0.0.1",
            h => h,
        };
        format!("http://{host}:{}", self.porta_api)
    }

    /// Lista de problemas; vazia quando dá para salvar.
    pub fn validar(&self) -> Vec<String> {
        let mut erros = Vec::new();

        if self.unidade.trim().is_empty() {
            erros.push("Informe a unidade (ex.: kg).".into());
        }

        if self.host.trim().parse::<IpAddr>().is_err() && self.host.trim() != "localhost" {
            erros.push("Host da API inválido: use 127.0.0.1 (só este computador) ou 0.0.0.0 (toda a rede).".into());
        }
        if self.porta_api == 0 {
            erros.push("A porta da API deve ser entre 1 e 65535.".into());
        }
        for origem in self.origens() {
            if origem != "*" && !(origem.starts_with("http://") || origem.starts_with("https://")) {
                erros.push(format!("Origem \"{origem}\" inválida: use o endereço completo, ex.: https://meusistema.com.br"));
            }
        }

        if self.intervalo_atualizacao < 0.05 || self.intervalo_atualizacao > 10.0 {
            erros.push("O intervalo de leitura deve ser entre 0,05 e 10 segundos.".into());
        }
        if self.tolerancia_estabilidade < 0.0 {
            erros.push("A tolerância de estabilidade não pode ser negativa.".into());
        }
        if self.tempo_estabilidade < 0.0 || self.tempo_estabilidade > 30.0 {
            erros.push("O tempo de estabilidade deve ser entre 0 e 30 segundos.".into());
        }
        if self.casas_decimais > 4 {
            erros.push("Use no máximo 4 casas decimais.".into());
        }

        if self.sim_peso_maximo <= self.sim_peso_minimo {
            erros.push("Na simulação, o peso máximo deve ser maior que o mínimo.".into());
        }
        if self.sim_resolucao < 0.001 {
            erros.push("A resolução da simulação deve ser de pelo menos 0,001.".into());
        }

        if self.porta_com.trim().is_empty() {
            erros.push("Informe a porta serial (ex.: COM3).".into());
        }
        if self.baudrate == 0 {
            erros.push("Informe o baudrate (ex.: 9600).".into());
        }
        if !(5..=8).contains(&self.bytesize) {
            erros.push("Bits de dados (bytesize) deve ser 5, 6, 7 ou 8.".into());
        }
        if !PARIDADES.iter().any(|(p, _)| *p == self.paridade) {
            erros.push("Paridade deve ser N, E ou O (mark e space não são suportadas).".into());
        }
        if self.stopbits != 1 && self.stopbits != 2 {
            erros.push("Stop bits deve ser 1 ou 2.".into());
        }
        if self.timeout < 0.05 || self.timeout > 30.0 {
            erros.push("O timeout da serial deve ser entre 0,05 e 30 segundos.".into());
        }

        for (nome, valor) in [
            ("Comando de peso", &self.comando_peso),
            ("Terminador", &self.terminador),
            ("Comando de zero", &self.comando_zero),
            ("Comando de tara", &self.comando_tara),
        ] {
            if let Err(e) = interpretar_escapes(valor) {
                erros.push(format!("{nome}: {e}"));
            }
        }
        if self.fator_divisao == 0.0 {
            erros.push("O fator de divisão não pode ser zero.".into());
        }
        if let Err(e) = self.protocolo() {
            erros.push(e);
        }

        if !NIVEIS_LOG.contains(&self.log_nivel.as_str()) {
            erros.push("Nível de log deve ser DEBUG, INFO, WARNING ou ERROR.".into());
        }

        erros
    }

    pub fn protocolo(&self) -> Result<Protocolo, String> {
        Protocolo::novo(
            &self.protocolo_regex,
            &self.protocolo_regex_estavel,
            self.fator_divisao,
            &self.unidade,
            self.casas_decimais,
        )
    }

    pub fn parametros_serial(&self) -> ParametrosSerial {
        ParametrosSerial {
            porta: self.porta_com.trim().to_string(),
            baudrate: self.baudrate,
            bytesize: self.bytesize,
            paridade: self.paridade.chars().next().unwrap_or('N'),
            stopbits: self.stopbits,
            timeout: Duration::from_secs_f64(self.timeout.clamp(0.05, 30.0)),
            comando_peso: interpretar_escapes(&self.comando_peso).unwrap_or_default(),
            terminador: interpretar_escapes(&self.terminador).unwrap_or_default(),
            comando_zero: interpretar_escapes(&self.comando_zero).unwrap_or_default(),
            comando_tara: interpretar_escapes(&self.comando_tara).unwrap_or_default(),
        }
    }

    /// Corrige o que veio fora da faixa de um config.toml editado à mão, para
    /// o agente abrir mesmo assim. A tela de Configurações mostra o problema.
    pub fn saneado(mut self) -> Self {
        let padrao = Config::default();
        if self.intervalo_atualizacao < 0.05 {
            self.intervalo_atualizacao = padrao.intervalo_atualizacao;
        }
        if self.fator_divisao == 0.0 {
            self.fator_divisao = 1.0;
        }
        if self.sim_resolucao <= 0.0 {
            self.sim_resolucao = padrao.sim_resolucao;
        }
        if self.porta_api == 0 {
            self.porta_api = padrao.porta_api;
        }
        self.casas_decimais = self.casas_decimais.min(4);
        self
    }
}

/// Converte o que se escreve na configuração ("\r\n", "\x05") em bytes.
pub fn interpretar_escapes(texto: &str) -> Result<Vec<u8>, String> {
    let mut saida = Vec::new();
    let mut chars = texto.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            saida.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('r') => saida.push(b'\r'),
            Some('n') => saida.push(b'\n'),
            Some('t') => saida.push(b'\t'),
            Some('0') => saida.push(0),
            Some('\\') => saida.push(b'\\'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                let byte = u8::from_str_radix(&hex, 16)
                    .map_err(|_| format!("\\x{hex} não é um byte válido; use dois dígitos hexadecimais, ex.: \\x05"))?;
                saida.push(byte);
            }
            Some(outro) => return Err(format!("escape \\{outro} desconhecido; use \\r, \\n, \\t, \\0, \\\\ ou \\xHH")),
            None => return Err("termina com \\ sozinha".into()),
        }
    }
    Ok(saida)
}

// ------------------------------------------------------------------ arquivo

pub fn diretorio_dados() -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        if dir.join("config.toml").is_file() {
            return dir;
        }
    }
    dirs::config_dir()
        .map(|d| d.join("BalancaAgent"))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn caminho_config() -> PathBuf {
    diretorio_dados().join("config.toml")
}

pub fn caminho_log() -> PathBuf {
    diretorio_dados().join("logs").join("balanca.log")
}

/// Lê a configuração. Arquivo ausente vira o padrão (e é criado); arquivo
/// ilegível vira o padrão também, com o motivo devolvido para ir ao log —
/// o original é preservado como config.toml.invalido.
pub fn carregar() -> (Config, Option<String>) {
    let caminho = caminho_config();
    match std::fs::read_to_string(&caminho) {
        Ok(texto) => match toml::from_str::<Config>(&texto) {
            Ok(cfg) => (cfg.saneado(), None),
            Err(e) => {
                let copia = caminho.with_extension("toml.invalido");
                let _ = std::fs::copy(&caminho, &copia);
                (
                    Config::default(),
                    Some(format!(
                        "config.toml ilegível ({e}); usando o padrão. O arquivo original foi copiado para {}",
                        copia.display()
                    )),
                )
            }
        },
        Err(_) => {
            let cfg = Config::default();
            let aviso = salvar(&cfg).err().map(|e| format!("Não foi possível criar {}: {e}", caminho.display()));
            (cfg, aviso)
        }
    }
}

pub fn salvar(cfg: &Config) -> std::io::Result<()> {
    let caminho = caminho_config();
    if let Some(dir) = caminho.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let corpo = toml::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    let texto = format!(
        "# Configuração do Agente de Balança.\n\
         # Prefira editar pela janela do agente (aba Configurações): ela valida\n\
         # antes de salvar. Se editar à mão, reabra o agente para aplicar.\n\n{corpo}"
    );
    // Grava num temporário e renomeia: uma queda de energia no meio da
    // gravação não deixa um config.toml pela metade.
    let temporario = caminho.with_extension("toml.tmp");
    std::fs::write(&temporario, texto)?;
    std::fs::rename(&temporario, &caminho)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn padrao_e_valido() {
        assert_eq!(Config::default().validar(), Vec::<String>::new());
    }

    #[test]
    fn toml_ida_e_volta() {
        let cfg = Config { modo_balanca: Modo::Real, protocolo_regex_estavel: "^ST".into(), ..Config::default() };
        let texto = toml::to_string_pretty(&cfg).unwrap();
        assert!(texto.contains("modo_balanca = \"real\""));
        assert_eq!(toml::from_str::<Config>(&texto).unwrap(), cfg);
    }

    #[test]
    fn campo_ausente_usa_o_padrao() {
        let cfg: Config = toml::from_str("porta_api = 5050").unwrap();
        assert_eq!(cfg.porta_api, 5050);
        assert_eq!(cfg.baudrate, 9600);
    }

    #[test]
    fn escapes() {
        assert_eq!(interpretar_escapes("P\\r\\n").unwrap(), b"P\r\n");
        assert_eq!(interpretar_escapes("\\x05").unwrap(), vec![5]);
        assert_eq!(interpretar_escapes("").unwrap(), Vec::<u8>::new());
        assert!(interpretar_escapes("\\q").is_err());
        assert!(interpretar_escapes("\\xZZ").is_err());
    }

    #[test]
    fn validacao_aponta_problemas() {
        let cfg = Config {
            protocolo_regex: "(\\d+".into(),
            sim_peso_maximo: 0.0,
            origens_permitidas: "meusistema.com".into(),
            stopbits: 3,
            ..Config::default()
        };
        let erros = cfg.validar().join(" | ");
        assert!(erros.contains("Regex do peso inválida"), "{erros}");
        assert!(erros.contains("peso máximo"), "{erros}");
        assert!(erros.contains("meusistema.com"), "{erros}");
        assert!(erros.contains("Stop bits"), "{erros}");
    }

    #[test]
    fn origens_e_url() {
        let mut cfg = Config { origens_permitidas: "https://a.com/, http://b.com\nhttp://c.com".into(), ..Config::default() };
        assert_eq!(cfg.origens(), vec!["https://a.com", "http://b.com", "http://c.com"]);
        cfg.host = "0.0.0.0".into();
        cfg.porta_api = 5050;
        assert_eq!(cfg.url_para_o_sistema(), "http://127.0.0.1:5050");
    }
}
