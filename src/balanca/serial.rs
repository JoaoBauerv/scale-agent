//! Balança física ligada em porta serial (USB/COM).
//!
//! Cada fabricante usa um formato de frame diferente, então nada de protocolo
//! está fixo no código. O que muda de balança para balança vem da
//! configuração: comando para pedir o peso, terminador do frame, regex que
//! extrai o número, regex de estabilidade, fator de divisão e comandos de
//! zero e tara.
//!
//! Quando a balança transmite sozinha, os frames se acumulam no buffer da
//! porta entre uma leitura e outra. Ler o mais antigo deixaria o visor
//! atrasado, então o agente lê tudo o que chegou e usa o ÚLTIMO frame
//! completo, e descarta o pedaço de frame que estava no meio do caminho
//! quando a porta foi aberta.

use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Serialize;
use serde_json::json;
use serialport::{ClearBuffer, DataBits, Parity, SerialPort, StopBits};

use super::{arredondar, Balanca, LeituraPeso};
use crate::erro::{Codigo, ErroBalanca};

/// Como interpretar o frame que a balança envia.
#[derive(Debug, Clone)]
pub struct Protocolo {
    regex: Regex,
    regex_estavel: Option<Regex>,
    fator_divisao: f64,
    unidade: String,
    casas_decimais: u32,
}

impl Protocolo {
    pub fn novo(
        regex: &str,
        regex_estavel: &str,
        fator_divisao: f64,
        unidade: &str,
        casas_decimais: u32,
    ) -> Result<Self, String> {
        let regex = Regex::new(regex).map_err(|e| format!("Regex do peso inválida: {e}"))?;
        if regex.captures_len() < 2 {
            return Err("A regex do peso precisa de um grupo entre parênteses, ex.: ([-+]?\\d+[.,]?\\d*)".into());
        }
        let regex_estavel = if regex_estavel.trim().is_empty() {
            None
        } else {
            Some(Regex::new(regex_estavel).map_err(|e| format!("Regex de estabilidade inválida: {e}"))?)
        };
        Ok(Self {
            regex,
            regex_estavel,
            fator_divisao: if fator_divisao == 0.0 { 1.0 } else { fator_divisao },
            unidade: unidade.to_string(),
            casas_decimais,
        })
    }

    /// Converte um frame cru numa leitura. Bytes fora do ASCII são ignorados.
    pub fn interpretar(&self, bruto: &[u8]) -> Result<LeituraPeso, ErroBalanca> {
        let frame: String = bruto.iter().filter(|b| b.is_ascii()).map(|&b| b as char).collect();
        let frame = frame.trim();

        let achado = self.regex.captures(frame).ok_or_else(|| {
            ErroBalanca::novo(
                Codigo::DadosInvalidos,
                format!("Frame recebido não bate com a regex do peso. Recebido: {frame:?}"),
            )
        })?;

        let texto = achado
            .name("peso")
            .or_else(|| achado.get(1))
            .map(|m| m.as_str())
            .unwrap_or("");

        let peso: f64 = texto.trim().replace(',', ".").parse().map_err(|_| {
            ErroBalanca::novo(
                Codigo::DadosInvalidos,
                format!("Não foi possível converter o peso do frame {frame:?}"),
            )
        })?;

        Ok(LeituraPeso {
            peso: arredondar(peso / self.fator_divisao, self.casas_decimais),
            unidade: self.unidade.clone(),
            estavel: self.regex_estavel.as_ref().map(|r| r.is_match(frame)),
        })
    }
}

/// Parâmetros da porta e dos comandos, já com os escapes convertidos em bytes.
#[derive(Debug, Clone)]
pub struct ParametrosSerial {
    pub porta: String,
    pub baudrate: u32,
    pub bytesize: u8,
    pub paridade: char,
    pub stopbits: u8,
    pub timeout: Duration,
    pub comando_peso: Vec<u8>,
    pub terminador: Vec<u8>,
    pub comando_zero: Vec<u8>,
    pub comando_tara: Vec<u8>,
}

pub struct BalancaSerial {
    p: ParametrosSerial,
    protocolo: Protocolo,
    conexao: Option<Box<dyn SerialPort>>,
    buffer: Vec<u8>,
    /// Logo após abrir a porta o primeiro frame pode estar cortado no meio.
    sincronizar: bool,
    ultimo_frame: Option<String>,
}

impl BalancaSerial {
    pub fn nova(p: ParametrosSerial, protocolo: Protocolo) -> Self {
        Self { p, protocolo, conexao: None, buffer: Vec::new(), sincronizar: true, ultimo_frame: None }
    }

    fn fechar(&mut self) {
        self.conexao = None;
        self.buffer.clear();
    }

    fn perdeu(&mut self, contexto: &str, e: impl std::fmt::Display) -> ErroBalanca {
        self.fechar();
        ErroBalanca::novo(Codigo::BalancaOffline, format!("{contexto}: {e}"))
    }

    fn ler_frame(&mut self) -> Result<Vec<u8>, ErroBalanca> {
        let prazo = Instant::now() + self.p.timeout;
        let mut pedaco = [0u8; 256];

        if !self.p.comando_peso.is_empty() {
            // Balança que só responde quando perguntada: descarta o que sobrou
            // e pergunta de novo, então a resposta é sempre a mais nova.
            let comando = self.p.comando_peso.clone();
            let porta = self.conexao.as_mut().ok_or_else(|| ErroBalanca::padrao(Codigo::BalancaOffline))?;
            let _ = porta.clear(ClearBuffer::Input);
            if let Err(e) = porta.write_all(&comando).and_then(|_| porta.flush()) {
                return Err(self.perdeu("Falha ao enviar o comando de peso", e));
            }
            self.buffer.clear();
            self.sincronizar = false;
        }

        loop {
            if self.p.terminador.is_empty() {
                // Sem terminador: o que chegar numa leitura é o frame.
                if !self.buffer.is_empty() {
                    return Ok(std::mem::take(&mut self.buffer));
                }
            } else if let Some(frame) = extrair_ultimo_frame(&mut self.buffer, &self.p.terminador, &mut self.sincronizar) {
                return Ok(frame);
            }

            if Instant::now() >= prazo {
                return Err(ErroBalanca::novo(
                    Codigo::Timeout,
                    format!(
                        "Nenhum frame completo recebido da balança em {:.1}s (porta {})",
                        self.p.timeout.as_secs_f64(),
                        self.p.porta
                    ),
                ));
            }

            let porta = self.conexao.as_mut().ok_or_else(|| ErroBalanca::padrao(Codigo::BalancaOffline))?;
            match porta.read(&mut pedaco) {
                Ok(0) => {}
                Ok(n) => {
                    self.buffer.extend_from_slice(&pedaco[..n]);
                    // Sem terminador nenhum, o buffer não pode crescer para sempre.
                    if self.buffer.len() > 8192 {
                        let excesso = self.buffer.len() - 1024;
                        self.buffer.drain(..excesso);
                    }
                }
                Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(self.perdeu("Comunicação perdida com a balança", e)),
            }
        }
    }

    fn enviar_comando(&mut self, nome: &str) -> Result<(), ErroBalanca> {
        let comando = if nome == "ZERO" { self.p.comando_zero.clone() } else { self.p.comando_tara.clone() };
        if comando.is_empty() {
            return Err(ErroBalanca::novo(
                Codigo::ComandoNaoSuportado,
                format!("Comando {nome} não configurado (Configurações → Protocolo)"),
            ));
        }
        let porta = self.conexao.as_mut().ok_or_else(|| ErroBalanca::padrao(Codigo::BalancaOffline))?;
        if let Err(e) = porta.write_all(&comando).and_then(|_| porta.flush()) {
            return Err(self.perdeu(&format!("Falha ao enviar o comando {nome}"), e));
        }
        Ok(())
    }
}

/// Tira do buffer o último frame completo (não vazio) e devolve sem o
/// terminador. O que vem depois do último terminador fica para a próxima.
///
/// Com `sincronizar`, tudo antes do primeiro terminador é descartado: é o
/// resto de um frame que começou antes de a porta ser aberta.
pub fn extrair_ultimo_frame(buffer: &mut Vec<u8>, terminador: &[u8], sincronizar: &mut bool) -> Option<Vec<u8>> {
    if terminador.is_empty() {
        return None;
    }

    if *sincronizar {
        let pos = encontrar(buffer, terminador, 0)?;
        buffer.drain(..pos + terminador.len());
        *sincronizar = false;
    }

    let mut inicio = 0;
    let mut ultimo: Option<(usize, usize)> = None;
    let mut fim_consumido = 0;
    while let Some(pos) = encontrar(buffer, terminador, inicio) {
        if buffer[inicio..pos].iter().any(|b| !b.is_ascii_whitespace()) {
            ultimo = Some((inicio, pos));
        }
        inicio = pos + terminador.len();
        fim_consumido = inicio;
    }

    let frame = ultimo.map(|(a, b)| buffer[a..b].to_vec());
    if fim_consumido > 0 {
        buffer.drain(..fim_consumido);
    }
    frame
}

fn encontrar(buffer: &[u8], agulha: &[u8], desde: usize) -> Option<usize> {
    if desde >= buffer.len() || agulha.len() > buffer.len() - desde {
        return None;
    }
    buffer[desde..].windows(agulha.len()).position(|j| j == agulha).map(|p| p + desde)
}

/// Mostra um frame cru de forma legível: "ST,GS,+125.40kg\r\n".
pub fn frame_legivel(bruto: &[u8]) -> String {
    let mut s = String::new();
    for &b in bruto {
        match b {
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02X}")),
        }
    }
    s
}

impl Balanca for BalancaSerial {
    fn modo(&self) -> &'static str {
        "real"
    }

    fn conectar(&mut self) -> Result<(), ErroBalanca> {
        self.fechar();

        let data_bits = match self.p.bytesize {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        };
        let paridade = match self.p.paridade {
            'E' => Parity::Even,
            'O' => Parity::Odd,
            _ => Parity::None,
        };
        let stop = if self.p.stopbits == 2 { StopBits::Two } else { StopBits::One };

        // Timeout curto por leitura; o prazo total do frame é controlado em
        // ler_frame(), para poder juntar pedaços que chegam aos poucos.
        let fatia = self.p.timeout.min(Duration::from_millis(100)).max(Duration::from_millis(10));

        let porta = serialport::new(&self.p.porta, self.p.baudrate)
            .data_bits(data_bits)
            .parity(paridade)
            .stop_bits(stop)
            .timeout(fatia)
            .open()
            .map_err(|e| {
                ErroBalanca::novo(
                    Codigo::PortaIndisponivel,
                    format!("Não foi possível abrir a porta {}: {e}", self.p.porta),
                )
            })?;

        self.conexao = Some(porta);
        self.sincronizar = true;
        Ok(())
    }

    fn desconectar(&mut self) {
        self.fechar();
    }

    fn esta_conectada(&self) -> bool {
        self.conexao.is_some()
    }

    fn ler_peso(&mut self) -> Result<LeituraPeso, ErroBalanca> {
        if self.conexao.is_none() {
            return Err(ErroBalanca::padrao(Codigo::BalancaOffline));
        }
        let frame = self.ler_frame()?;
        self.ultimo_frame = Some(frame_legivel(&frame));
        self.protocolo.interpretar(&frame)
    }

    fn zerar(&mut self) -> Result<(), ErroBalanca> {
        self.enviar_comando("ZERO")
    }

    fn tarar(&mut self) -> Result<(), ErroBalanca> {
        self.enviar_comando("TARA")
    }

    fn descricao(&self) -> serde_json::Value {
        json!({
            "modo": self.modo(),
            "porta": self.p.porta,
            "baudrate": self.p.baudrate,
            "bytesize": self.p.bytesize,
            "paridade": self.p.paridade.to_string(),
            "stopbits": self.p.stopbits,
            "timeout": self.p.timeout.as_secs_f64(),
        })
    }

    fn ultimo_frame(&self) -> Option<String> {
        self.ultimo_frame.clone()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PortaInfo {
    pub porta: String,
    pub descricao: String,
    pub fabricante: String,
}

/// Portas seriais encontradas na máquina (ajuda a descobrir a COM certa).
pub fn listar_portas() -> Vec<PortaInfo> {
    let mut portas: Vec<PortaInfo> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let (descricao, fabricante) = match p.port_type {
                serialport::SerialPortType::UsbPort(usb) => (
                    usb.product.unwrap_or_else(|| "Dispositivo USB".into()),
                    usb.manufacturer.unwrap_or_default(),
                ),
                serialport::SerialPortType::BluetoothPort => ("Bluetooth".into(), String::new()),
                serialport::SerialPortType::PciPort => ("Porta serial".into(), String::new()),
                serialport::SerialPortType::Unknown => (String::new(), String::new()),
            };
            PortaInfo { porta: p.port_name, descricao, fabricante }
        })
        .collect();
    portas.sort_by(|a, b| a.porta.cmp(&b.porta));
    portas
}

#[cfg(test)]
mod testes {
    use super::*;

    fn protocolo(regex: &str, estavel: &str, fator: f64) -> Protocolo {
        Protocolo::novo(regex, estavel, fator, "kg", 2).unwrap()
    }

    #[test]
    fn interpreta_frame() {
        let p = protocolo(r"([-+]?\d+[.,]?\d*)", "", 1.0);
        let l = p.interpretar(b"ST,GS,+  125.40 kg\r\n").unwrap();
        assert_eq!(l.peso, 125.40);
        assert_eq!(l.estavel, None);
    }

    #[test]
    fn aplica_fator_divisao() {
        let p = protocolo(r"(\d+)", "", 100.0);
        assert_eq!(p.interpretar(b"012540\r\n").unwrap().peso, 125.40);
    }

    #[test]
    fn le_flag_de_estabilidade() {
        let p = protocolo(r"([-+]?\d+[.,]?\d*)", "^ST", 1.0);
        assert_eq!(p.interpretar(b"ST,GS,+  125.40 kg\r\n").unwrap().estavel, Some(true));
        assert_eq!(p.interpretar(b"US,GS,+  125.40 kg\r\n").unwrap().estavel, Some(false));
    }

    #[test]
    fn rejeita_frame_invalido() {
        let p = protocolo(r"(\d+\.\d+)", "", 1.0);
        let erro = p.interpretar(b"ERRO\r\n").unwrap_err();
        assert_eq!(erro.codigo, Codigo::DadosInvalidos);
    }

    #[test]
    fn aceita_virgula_sinal_e_grupo_nomeado() {
        let p = protocolo(r"PESO=(?P<peso>[-+]?\d+,\d+)", "", 1.0);
        assert_eq!(p.interpretar(b"PESO=-12,5\r").unwrap().peso, -12.5);
        let p = protocolo(r"([-+]?\d+[.,]?\d*)", "", 1.0);
        assert_eq!(p.interpretar(b"+125.").unwrap().peso, 125.0);
    }

    #[test]
    fn regex_sem_grupo_e_recusada() {
        assert!(Protocolo::novo(r"\d+", "", 1.0, "kg", 2).is_err());
        assert!(Protocolo::novo(r"(\d+", "", 1.0, "kg", 2).is_err());
    }

    #[test]
    fn usa_o_ultimo_frame_completo() {
        let mut buf = b"ST,+1.00\r\nST,+2.00\r\nST,+3.0".to_vec();
        let mut sinc = false;
        let frame = extrair_ultimo_frame(&mut buf, b"\r\n", &mut sinc).unwrap();
        assert_eq!(frame, b"ST,+2.00");
        assert_eq!(buf, b"ST,+3.0", "o frame incompleto fica para a próxima leitura");
    }

    #[test]
    fn descarta_frame_cortado_ao_abrir_a_porta() {
        let mut buf = b"5.40 kg\r\nST,+  7.10 kg\r\n".to_vec();
        let mut sinc = true;
        assert_eq!(extrair_ultimo_frame(&mut buf, b"\r\n", &mut sinc).unwrap(), b"ST,+  7.10 kg");
        assert!(!sinc);

        let mut buf = b"pedaco sem fim".to_vec();
        let mut sinc = true;
        assert!(extrair_ultimo_frame(&mut buf, b"\r\n", &mut sinc).is_none());
        assert!(sinc, "continua esperando o primeiro terminador");
    }

    #[test]
    fn ignora_frames_vazios() {
        let mut buf = b"ST,+4.00\r\n\r\n  \r\n".to_vec();
        let mut sinc = false;
        assert_eq!(extrair_ultimo_frame(&mut buf, b"\r\n", &mut sinc).unwrap(), b"ST,+4.00");
        assert!(buf.is_empty());
    }

    #[test]
    fn porta_inexistente_da_porta_indisponivel() {
        let mut b = BalancaSerial::nova(
            ParametrosSerial {
                porta: "COM_TESTE_INEXISTENTE".into(),
                baudrate: 9600,
                bytesize: 8,
                paridade: 'N',
                stopbits: 1,
                timeout: Duration::from_millis(200),
                comando_peso: vec![],
                terminador: b"\r\n".to_vec(),
                comando_zero: vec![],
                comando_tara: vec![],
            },
            protocolo(r"(\d+)", "", 1.0),
        );
        assert_eq!(b.conectar().unwrap_err().codigo, Codigo::PortaIndisponivel);
        assert_eq!(b.ler_peso().unwrap_err().codigo, Codigo::BalancaOffline);
    }

    #[test]
    fn frame_legivel_mostra_controle() {
        assert_eq!(frame_legivel(b"ST,+1.0\r\n\x02"), "ST,+1.0\\r\\n\\x02");
    }
}
