//! Janela do agente.
//!
//! - Balança: visor ao vivo e os comandos (gerar peso, tara, zero, modo).
//! - Configurações: todos os parâmetros, validados antes de salvar e
//!   aplicados na hora, sem reiniciar.
//! - Integração: o endereço, as rotas e os campos que o sistema web usa para
//!   falar com este agente.
//! - Log: as últimas mensagens.
//!
//! Comandos que falam com a balança rodam fora da thread da janela: com a
//! balança física, um comando pode esperar até o timeout da porta.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText, Ui};
use log::{info, warn};
use rand::Rng;

use crate::api::exemplo_resposta_peso;
use crate::autostart;
use crate::balanca::serial::{listar_portas, PortaInfo};
use crate::config::{self, interpretar_escapes, Config, Modo, BAUDRATES, NIVEIS_LOG, PARIDADES};
use crate::registro;
use crate::servico::{EstadoPesagem, ServicoPesagem};
use crate::servidor::ServidorApi;

const VERDE: Color32 = Color32::from_rgb(25, 135, 84);
const AMARELO: Color32 = Color32::from_rgb(214, 150, 0);
const VERMELHO: Color32 = Color32::from_rgb(220, 53, 69);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Aba {
    Balanca,
    Configuracoes,
    Integracao,
    Log,
}

pub struct App {
    servico: Arc<ServicoPesagem>,
    servidor: Arc<Mutex<Option<ServidorApi>>>,
    erro_servidor: Option<String>,

    salva: Config,
    form: Config,
    erros: Vec<String>,

    aba: Aba,
    aviso: Option<(bool, String, Instant)>,
    resultados_tx: Sender<(bool, String)>,
    resultados_rx: Receiver<(bool, String)>,

    portas: Vec<PortaInfo>,
    frame_teste: String,
    resultado_teste: Option<Result<String, String>>,
    peso_gerar: String,
    mostrar_token: bool,
    minimizar_ao_abrir: bool,
}

impl App {
    pub fn new(
        servico: Arc<ServicoPesagem>,
        servidor: Arc<Mutex<Option<ServidorApi>>>,
        erro_servidor: Option<String>,
        config: Config,
        minimizado: bool,
        aviso_inicial: Option<String>,
        aba_inicial: Option<&str>,
    ) -> Self {
        let (resultados_tx, resultados_rx) = mpsc::channel();
        Self {
            servico,
            servidor,
            erro_servidor,
            salva: config.clone(),
            form: config,
            erros: Vec::new(),
            aba: match aba_inicial {
                Some("configuracoes") => Aba::Configuracoes,
                Some("integracao") => Aba::Integracao,
                Some("log") => Aba::Log,
                _ => Aba::Balanca,
            },
            aviso: aviso_inicial.map(|m| (false, m, Instant::now())),
            resultados_tx,
            resultados_rx,
            portas: listar_portas(),
            frame_teste: String::new(),
            resultado_teste: None,
            peso_gerar: String::new(),
            mostrar_token: false,
            minimizar_ao_abrir: minimizado,
        }
    }

    fn avisar(&mut self, ok: bool, msg: impl Into<String>) {
        self.aviso = Some((ok, msg.into(), Instant::now()));
    }

    /// Roda um comando da balança numa thread e devolve o resultado como aviso.
    fn executar(&self, f: impl FnOnce(&ServicoPesagem) -> Result<String, String> + Send + 'static) {
        let servico = Arc::clone(&self.servico);
        let tx = self.resultados_tx.clone();
        std::thread::spawn(move || {
            let r = f(&servico);
            let _ = tx.send(match r {
                Ok(m) => (true, m),
                Err(m) => (false, m),
            });
        });
    }

    fn salvar(&mut self) {
        self.erros = self.form.validar();
        if !self.erros.is_empty() {
            self.avisar(false, "Corrija os campos indicados antes de salvar.");
            return;
        }
        if let Err(e) = config::salvar(&self.form) {
            self.avisar(false, format!("Não foi possível gravar {}: {e}", config::caminho_config().display()));
            return;
        }

        let mut avisos = Vec::new();
        if self.form.iniciar_com_windows != autostart::ativo() {
            if let Err(e) = autostart::definir(self.form.iniciar_com_windows) {
                avisos.push(e);
            }
        }

        registro::definir_nivel(&self.form.log_nivel);
        self.servico.reconfigurar(self.form.clone());

        let api_mudou = self.form.host != self.salva.host
            || self.form.porta_api != self.salva.porta_api
            || self.form.origens_permitidas != self.salva.origens_permitidas
            || self.form.permitir_rede != self.salva.permitir_rede
            || self.form.api_token != self.salva.api_token;
        if api_mudou || self.erro_servidor.is_some() {
            let mut servidor = self.servidor.lock().unwrap();
            if let Some(mut antigo) = servidor.take() {
                antigo.parar();
            }
            match ServidorApi::iniciar(&self.form, Arc::clone(&self.servico)) {
                Ok(novo) => {
                    *servidor = Some(novo);
                    self.erro_servidor = None;
                }
                Err(e) => {
                    warn!("{e}");
                    avisos.push(e.clone());
                    self.erro_servidor = Some(e);
                }
            }
        }

        info!("Configurações salvas em {}", config::caminho_config().display());
        self.salva = self.form.clone();
        if avisos.is_empty() {
            self.avisar(true, "Configurações salvas e aplicadas.");
        } else {
            self.avisar(false, format!("Configurações salvas, mas: {}", avisos.join(" ")));
        }
    }

    // ------------------------------------------------------------ cabeçalho

    fn barra_superior(&mut self, ui: &mut Ui, estado: &EstadoPesagem) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Scale Agent").size(18.0).strong());
            ui.label(RichText::new("agente de balança").weak());
            ui.separator();

            let (cor, texto) = if !estado.conectada {
                (VERMELHO, "Desconectada")
            } else if estado.modo == "simulado" {
                (VERDE, "Simulação ativa")
            } else {
                (VERDE, "Balança conectada")
            };
            bolinha(ui, cor);
            ui.label(texto);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &self.erro_servidor {
                Some(e) => {
                    ui.label(RichText::new("API parada").color(VERMELHO).strong()).on_hover_text(e);
                }
                None => {
                    let url = self.salva.url_para_o_sistema();
                    ui.label(RichText::new(format!("API: {url}")).color(VERDE));
                }
            });
        });
    }

    fn mostrar_aviso(&mut self, ui: &mut Ui) {
        if let Some((ok, msg, quando)) = &self.aviso {
            if quando.elapsed() > Duration::from_secs(8) {
                self.aviso = None;
                return;
            }
            let cor = if *ok { VERDE } else { VERMELHO };
            egui::Frame::none()
                .fill(cor.gamma_multiply(0.15))
                .stroke(egui::Stroke::new(1.0_f32, cor))
                .rounding(6.0_f32)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(msg.as_str()).color(cor));
                });
            ui.add_space(8.0);
        }
    }

    // --------------------------------------------------------------- balança

    fn aba_balanca(&mut self, ui: &mut Ui, estado: &EstadoPesagem) {
        let casas = self.salva.casas_decimais;
        let simulado = estado.modo == "simulado";

        cartao(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if simulado { "BALANÇA SIMULADA" } else { "BALANÇA REAL" }).weak());
                let cor = if !estado.conectada {
                    ui.visuals().weak_text_color()
                } else if estado.estavel {
                    VERDE
                } else {
                    AMARELO
                };
                ui.label(
                    RichText::new(formatar_peso(estado.peso, casas, &estado.unidade))
                        .size(72.0)
                        .monospace()
                        .strong()
                        .color(cor),
                );
                let situacao = if !estado.conectada {
                    "Sem leitura"
                } else if estado.estavel {
                    "Peso estável - pronto para capturar"
                } else {
                    "Estabilizando..."
                };
                ui.label(RichText::new(situacao).color(cor));
            });
        });

        if let (Some(erro), Some(codigo)) = (&estado.erro, &estado.codigo_erro) {
            ui.add_space(8.0);
            egui::Frame::none()
                .fill(VERMELHO.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0_f32, VERMELHO))
                .rounding(6.0_f32)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("[{codigo}] {erro}")).color(VERMELHO).strong());
                    if let Some(dica) = dica_do_erro(codigo) {
                        ui.label(dica);
                    }
                });
        }

        ui.add_space(10.0);
        cartao(ui, |ui| {
            ui.label(RichText::new("Comandos").strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if simulado {
                    if ui.button("Gerar peso").clicked() {
                        self.executar(|s| s.gerar_peso(None).map(|p| format!("Peso simulado: {p:.2}")));
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.peso_gerar).desired_width(80.0).hint_text("ex.: 125,40"));
                    if ui.button("Colocar este peso").clicked() {
                        match self.peso_gerar.trim().replace(',', ".").parse::<f64>() {
                            Ok(v) => self.executar(move |s| s.gerar_peso(Some(v)).map(|p| format!("Peso simulado: {p:.2}"))),
                            Err(_) => self.avisar(false, "Digite um número, ex.: 125,40"),
                        }
                    }
                    ui.separator();
                }
                if ui.button("Aplicar tara").clicked() {
                    self.executar(|s| s.tarar().map(|_| "Tara aplicada".to_string()).map_err(|e| e.mensagem));
                }
                if ui.button("Zerar balança").clicked() {
                    self.executar(|s| s.zerar().map(|_| "Balança zerada".to_string()).map_err(|e| e.mensagem));
                }
                ui.separator();
                let (rotulo, alvo) = if simulado { ("Usar balança real", "real") } else { ("Voltar para a simulada", "simulada") };
                if ui.button(RichText::new(rotulo).strong()).clicked() {
                    self.executar(move |s| s.definir_modo(alvo).map(|e| format!("Modo: {}", e.modo)));
                }
            });
            ui.label(
                RichText::new("A troca de modo aqui vale até fechar o agente. Para fixar, mude em Configurações > Operação.")
                    .small()
                    .weak(),
            );
        });

        ui.add_space(10.0);
        cartao(ui, |ui| {
            ui.label(RichText::new("Conexão").strong());
            egui::Grid::new("conexao").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                let c = &estado.conexao;
                if simulado {
                    let num = |campo: &str| c[campo].as_f64();
                    let un = estado.unidade.as_str();
                    linha_info(
                        ui,
                        "Faixa sorteada",
                        &format!("{} a {}", formatar_peso(num("peso_minimo"), casas, un), formatar_peso(num("peso_maximo"), casas, un)),
                    );
                    linha_info(ui, "Resolução", &formatar_peso(num("resolucao"), 3, un));
                    linha_info(ui, "Tara aplicada", &formatar_peso(num("tara_aplicada"), casas, un));
                } else {
                    linha_info(ui, "Porta", c["porta"].as_str().unwrap_or("-"));
                    linha_info(
                        ui,
                        "Parâmetros",
                        &format!(
                            "{} baud, {} bits, paridade {}, {} stop bit(s)",
                            c["baudrate"],
                            c["bytesize"],
                            c["paridade"].as_str().unwrap_or("N"),
                            c["stopbits"]
                        ),
                    );
                    linha_info(ui, "Último frame recebido", estado.ultimo_frame.as_deref().unwrap_or("(nenhum ainda)"));
                }
                let agora = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                linha_info(ui, "Última leitura", &format!("há {:.1} s", (agora - estado.atualizado_em).max(0.0)));
            });
        });
    }

    // --------------------------------------------------------- configurações

    fn aba_configuracoes(&mut self, ui: &mut Ui, estado: &EstadoPesagem) {
        ui.label(
            RichText::new(format!("Arquivo: {}", config::caminho_config().display())).small().weak(),
        );
        ui.add_space(6.0);

        if !self.erros.is_empty() {
            egui::Frame::none()
                .fill(VERMELHO.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0_f32, VERMELHO))
                .rounding(6.0_f32)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Não foi possível salvar:").color(VERMELHO).strong());
                    for e in &self.erros {
                        ui.label(RichText::new(format!("- {e}")).color(VERMELHO));
                    }
                });
            ui.add_space(8.0);
        }

        let form = &mut self.form;

        secao(ui, "Operação", "Como o agente abre.", |ui| {
            linha(ui, "Modo ao abrir", "Simulada não precisa de equipamento: serve para testar o sistema.", |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut form.modo_balanca, Modo::Simulada, "Simulada");
                    ui.radio_value(&mut form.modo_balanca, Modo::Real, "Real (porta serial)");
                });
            });
            linha(ui, "Unidade", "Devolvida pela API e mostrada no sistema.", |ui| {
                ui.add(egui::TextEdit::singleline(&mut form.unidade).desired_width(80.0));
            });
        });

        let mostrar_token = &mut self.mostrar_token;
        secao(ui, "API local", "Onde o sistema web encontra o agente.", |ui| {
            linha(ui, "Host", "127.0.0.1 = só este computador (recomendado). 0.0.0.0 = toda a rede.", |ui| {
                ui.add(egui::TextEdit::singleline(&mut form.host).desired_width(140.0));
            });
            linha(ui, "Porta", "A mesma que vai no endereço configurado no sistema web.", |ui| {
                ui.add(egui::DragValue::new(&mut form.porta_api).range(1..=65535));
            });
            linha(
                ui,
                "Origens permitidas",
                "Endereço do sistema web, um por linha (ex.: https://meusistema.vercel.app). \
                 localhost e 127.0.0.1 já são liberados. * libera qualquer um (só para testes).",
                |ui| {
                    ui.add(egui::TextEdit::multiline(&mut form.origens_permitidas).desired_rows(3).desired_width(420.0));
                },
            );
            linha(ui, "Acesso pela rede", "Desligado: só aceita chamadas deste computador.", |ui| {
                ui.checkbox(&mut form.permitir_rede, "Permitir chamadas de outros computadores");
            });
            linha(
                ui,
                "Token da API",
                "Opcional. Protege tara, zero, modo e gerar peso (header X-API-Token). /peso e /captura não exigem.",
                |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut form.api_token).password(!*mostrar_token).desired_width(220.0));
                        ui.checkbox(mostrar_token, "mostrar");
                        if ui.button("Gerar").clicked() {
                            let mut rng = rand::thread_rng();
                            form.api_token = (0..24).map(|_| format!("{:x}", rng.gen_range(0..16))).collect();
                            *mostrar_token = true;
                        }
                    });
                },
            );
        });

        secao(ui, "Leitura e estabilidade", "Quando o peso é considerado parado.", |ui| {
            linha(ui, "Intervalo de leitura", "De quanto em quanto tempo o agente lê a balança.", |ui| {
                ui.add(egui::DragValue::new(&mut form.intervalo_atualizacao).range(0.05..=10.0).speed(0.05).fixed_decimals(2).suffix(" s"));
            });
            linha(ui, "Tolerância", "Variação máxima, na unidade, para o peso contar como parado.", |ui| {
                ui.add(egui::DragValue::new(&mut form.tolerancia_estabilidade).range(0.0..=1000.0).speed(0.01).fixed_decimals(3));
            });
            linha(ui, "Tempo parado", "Quanto tempo o peso precisa ficar parado para ser estável. 0 = sempre estável.", |ui| {
                ui.add(egui::DragValue::new(&mut form.tempo_estabilidade).range(0.0..=30.0).speed(0.1).fixed_decimals(1).suffix(" s"));
            });
            linha(ui, "Casas decimais", "", |ui| {
                ui.add(egui::DragValue::new(&mut form.casas_decimais).range(0..=4));
            });
        });

        secao(ui, "Balança simulada", "Usada no modo Simulada.", |ui| {
            linha(ui, "Faixa do peso sorteado", "Botão Gerar peso.", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut form.sim_peso_minimo).speed(1.0).fixed_decimals(2));
                    ui.label("a");
                    ui.add(egui::DragValue::new(&mut form.sim_peso_maximo).speed(1.0).fixed_decimals(2));
                });
            });
            linha(ui, "Peso inicial", "", |ui| {
                ui.add(egui::DragValue::new(&mut form.sim_peso_inicial).speed(1.0).fixed_decimals(2));
            });
            linha(ui, "Resolução", "Divisão da balança: com 0,10 o visor mostra 195,50, nunca 195,47.", |ui| {
                ui.add(egui::DragValue::new(&mut form.sim_resolucao).range(0.001..=100.0).speed(0.01).fixed_decimals(3));
            });
        });

        let portas = &self.portas;
        let mut atualizar_portas = false;
        secao(ui, "Porta serial", "Balança física. Os valores estão no manual do equipamento.", |ui| {
            linha(ui, "Porta", "No Windows, Gerenciador de Dispositivos > Portas (COM e LPT).", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.porta_com).desired_width(110.0));
                    egui::ComboBox::from_id_salt("portas").selected_text("Escolher...").show_ui(ui, |ui| {
                        if portas.is_empty() {
                            ui.label("Nenhuma porta encontrada");
                        }
                        for p in portas {
                            let texto = format!("{}  {} {}", p.porta, p.descricao, p.fabricante);
                            if ui.selectable_label(form.porta_com == p.porta, texto.trim()).clicked() {
                                form.porta_com = p.porta.clone();
                            }
                        }
                    });
                    if ui.button("Atualizar lista").clicked() {
                        atualizar_portas = true;
                    }
                });
            });
            linha(ui, "Baudrate", "", |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("baud").selected_text(form.baudrate.to_string()).show_ui(ui, |ui| {
                        for b in BAUDRATES {
                            ui.selectable_value(&mut form.baudrate, b, b.to_string());
                        }
                    });
                    ui.label("outro:");
                    ui.add(egui::DragValue::new(&mut form.baudrate).range(1..=4_000_000));
                });
            });
            linha(ui, "Bits de dados", "", |ui| {
                ui.horizontal(|ui| {
                    for b in 5..=8u8 {
                        ui.radio_value(&mut form.bytesize, b, b.to_string());
                    }
                });
            });
            linha(ui, "Paridade", "", |ui| {
                let atual = PARIDADES.iter().find(|(p, _)| *p == form.paridade).map(|(_, t)| *t).unwrap_or("?");
                egui::ComboBox::from_id_salt("paridade").selected_text(atual).show_ui(ui, |ui| {
                    for (p, texto) in PARIDADES {
                        ui.selectable_value(&mut form.paridade, p.to_string(), texto);
                    }
                });
            });
            linha(ui, "Stop bits", "", |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut form.stopbits, 1, "1");
                    ui.radio_value(&mut form.stopbits, 2, "2");
                });
            });
            linha(ui, "Timeout", "Quanto esperar por um frame completo antes de dar TIMEOUT.", |ui| {
                ui.add(egui::DragValue::new(&mut form.timeout).range(0.05..=30.0).speed(0.1).fixed_decimals(2).suffix(" s"));
            });
        });

        let frame_teste = &mut self.frame_teste;
        let resultado_teste = &mut self.resultado_teste;
        secao(
            ui,
            "Protocolo da balança física",
            "Como ler o que a balança envia. Comandos e terminador aceitam \\r \\n \\t \\0 \\\\ e \\xHH.",
            |ui| {
                linha(ui, "Comando de peso", "O que enviar para pedir o peso. Vazio se a balança transmite sozinha (o mais comum).", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.comando_peso).desired_width(160.0).font(egui::TextStyle::Monospace).hint_text("ex.: P\\r\\n"));
                });
                linha(ui, "Terminador", "Fim de cada frame. Vazio: cada leitura da porta é um frame.", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.terminador).desired_width(160.0).font(egui::TextStyle::Monospace));
                });
                linha(ui, "Regex do peso", "O grupo 1 (ou o grupo (?P<peso>...)) é o número.", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.protocolo_regex).desired_width(320.0).font(egui::TextStyle::Monospace));
                });
                linha(ui, "Regex de estabilidade", "Opcional. Ex.: ^ST quando a balança marca estável no frame. Vazio = o agente calcula.", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.protocolo_regex_estavel).desired_width(320.0).font(egui::TextStyle::Monospace));
                });
                linha(ui, "Fator de divisão", "Balança que manda o peso sem vírgula: 012540 com fator 100 = 125,40.", |ui| {
                    ui.add(egui::DragValue::new(&mut form.fator_divisao).range(0.0001..=1_000_000.0).speed(1.0));
                });
                linha(ui, "Comandos de zero e tara", "Vazios: o agente responde COMANDO_NAO_SUPORTADO.", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("zero");
                        ui.add(egui::TextEdit::singleline(&mut form.comando_zero).desired_width(100.0).font(egui::TextStyle::Monospace));
                        ui.label("tara");
                        ui.add(egui::TextEdit::singleline(&mut form.comando_tara).desired_width(100.0).font(egui::TextStyle::Monospace));
                    });
                });
                linha(
                    ui,
                    "Testar protocolo",
                    "Cole um frame (ou use o último recebido) e veja o que o agente entende, com os valores acima, antes de salvar.",
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(frame_teste)
                                    .desired_width(240.0)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("ST,GS,+  125.40 kg\\r\\n"),
                            );
                            if let Some(ultimo) = &estado.ultimo_frame {
                                if ui.button("Usar o último recebido").clicked() {
                                    *frame_teste = ultimo.clone();
                                }
                            }
                            if ui.button("Interpretar").clicked() {
                                *resultado_teste = Some(testar_protocolo(form, frame_teste));
                            }
                        });
                        match resultado_teste {
                            Some(Ok(t)) => {
                                ui.label(RichText::new(t.as_str()).color(VERDE).strong());
                            }
                            Some(Err(t)) => {
                                ui.label(RichText::new(t.as_str()).color(VERMELHO));
                            }
                            None => {}
                        }
                    },
                );
            },
        );

        secao(ui, "Log e Windows", "", |ui| {
            linha(ui, "Nível do log", "DEBUG registra cada leitura; use só para investigar.", |ui| {
                egui::ComboBox::from_id_salt("nivel").selected_text(form.log_nivel.clone()).show_ui(ui, |ui| {
                    for n in NIVEIS_LOG {
                        ui.selectable_value(&mut form.log_nivel, n.to_string(), n);
                    }
                });
            });
            linha(ui, "Inicialização", "Abre minimizado ao entrar no Windows, para o sistema sempre achar a balança.", |ui| {
                ui.checkbox(&mut form.iniciar_com_windows, "Iniciar com o Windows");
            });
        });

        if atualizar_portas {
            self.portas = listar_portas();
        }
    }

    fn rodape_configuracoes(&mut self, ui: &mut Ui) {
        let alterado = self.form != self.salva;
        ui.horizontal(|ui| {
            let salvar = egui::Button::new(RichText::new("Salvar e aplicar").strong().color(Color32::WHITE)).fill(VERDE);
            if ui.add_enabled(alterado, salvar).clicked() {
                self.salvar();
            }
            if ui.add_enabled(alterado, egui::Button::new("Descartar alterações")).clicked() {
                self.form = self.salva.clone();
                self.erros.clear();
            }
            if ui.button("Restaurar padrão").on_hover_text("Preenche os valores de fábrica. Só vale ao salvar.").clicked() {
                self.form = Config { iniciar_com_windows: self.form.iniciar_com_windows, ..Config::default() };
            }
            if ui.button("Abrir pasta").on_hover_text("Pasta do config.toml e dos logs").clicked() {
                abrir(&config::diretorio_dados().display().to_string());
            }
            if alterado {
                ui.label(RichText::new("Há alterações não salvas").color(AMARELO));
            }
        });
    }

    // ------------------------------------------------------------ integração

    fn aba_integracao(&mut self, ui: &mut Ui) {
        let cfg = &self.salva;
        let url = cfg.url_para_o_sistema();

        cartao(ui, |ui| {
            ui.label(RichText::new("Ligar ao sistema web").size(16.0).strong());
            ui.label(
                "O sistema web chama este agente pelo navegador do computador da balança. \
                 Use estes valores na configuração de balança do seu sistema.",
            );
            ui.add_space(6.0);

            let intervalo_ms = ((cfg.intervalo_atualizacao * 1000.0).round() as u32).max(300).to_string();
            let valores: [(&str, String); 9] = [
                ("Endereço do agente", url.clone()),
                ("Rota de leitura do peso", "/peso".into()),
                ("Rota de registro da captura", "/captura".into()),
                ("Intervalo entre leituras", format!("{intervalo_ms} ms")),
                ("Campo: Peso", "peso".into()),
                ("Campo: Peso estável", "estavel".into()),
                ("Campo: Unidade", "unidade".into()),
                ("Campo: Leitura válida", "sucesso".into()),
                ("Campo: Mensagem de erro", "erro".into()),
            ];
            egui::Grid::new("integracao").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                for (rotulo, valor) in &valores {
                    ui.label(*rotulo);
                    ui.label(RichText::new(valor.as_str()).monospace().strong());
                    if ui.small_button("Copiar").clicked() {
                        ui.output_mut(|o| o.copied_text = valor.clone());
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Copiar tudo").clicked() {
                    let texto = valores.iter().map(|(r, v)| format!("{r}: {v}")).collect::<Vec<_>>().join("\n");
                    ui.output_mut(|o| o.copied_text = texto);
                }
                if ui.button("Abrir a página do agente no navegador").clicked() {
                    abrir(&url);
                }
            });
        });

        ui.add_space(10.0);
        let origens = cfg.origens();
        cartao(ui, |ui| {
            ui.label(RichText::new("Sistema publicado na internet").strong());
            ui.label(
                "Quem lê a balança é o navegador do computador do balcão, não o servidor. Se o sistema estiver \
                 num endereço da internet (ex.: Vercel), inclua esse endereço em Configurações > API local > \
                 Origens permitidas, senão o navegador bloqueia a leitura.",
            );
            let liberadas = if origens.is_empty() { "só localhost".to_string() } else { origens.join(", ") };
            ui.label(RichText::new(format!("Liberadas hoje: localhost, 127.0.0.1, {liberadas}")).small().weak());
            if cfg.host != "127.0.0.1" && cfg.host != "localhost" {
                ui.label(
                    RichText::new("A API está aberta para a rede (host diferente de 127.0.0.1). Use isso só se precisar.")
                        .color(AMARELO),
                );
            }
        });

        ui.add_space(10.0);
        cartao(ui, |ui| {
            ui.label(RichText::new("O que GET /peso responde agora").strong());
            let json = serde_json::to_string_pretty(&exemplo_resposta_peso(&self.servico)).unwrap_or_default();
            ui.add(egui::TextEdit::multiline(&mut json.as_str()).font(egui::TextStyle::Monospace).desired_rows(7).desired_width(f32::INFINITY));
        });
    }

    // -------------------------------------------------------------------- log

    fn aba_log(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("Abrir arquivo de log").clicked() {
                abrir(&config::caminho_log().display().to_string());
            }
            if ui.button("Limpar a tela").clicked() {
                registro::limpar_memoria();
            }
            ui.label(RichText::new(config::caminho_log().display().to_string()).small().weak());
        });
        ui.separator();
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for l in registro::ultimas_linhas() {
                let cor = match l.nivel {
                    log::Level::Error => VERMELHO,
                    log::Level::Warn => AMARELO,
                    _ => ui.visuals().text_color(),
                };
                ui.label(RichText::new(format!("[{}] {:<5} {}", l.hora, l.nivel, l.texto)).monospace().color(cor));
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.minimizar_ao_abrir {
            self.minimizar_ao_abrir = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        while let Ok((ok, msg)) = self.resultados_rx.try_recv() {
            self.avisar(ok, msg);
        }
        ctx.request_repaint_after(Duration::from_millis(200));

        let estado = self.servico.estado();

        egui::TopBottomPanel::top("topo").show(ctx, |ui| self.barra_superior(ui, &estado));

        egui::SidePanel::left("abas").resizable(false).exact_width(170.0).show(ctx, |ui| {
            ui.add_space(8.0);
            for (aba, rotulo) in [
                (Aba::Balanca, "Balança"),
                (Aba::Configuracoes, "Configurações"),
                (Aba::Integracao, "Integração"),
                (Aba::Log, "Log"),
            ] {
                let texto = RichText::new(rotulo).size(15.0);
                if ui.add_sized([150.0, 32.0], egui::SelectableLabel::new(self.aba == aba, texto)).clicked() {
                    self.aba = aba;
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).small().weak());
            });
        });

        if self.aba == Aba::Configuracoes {
            egui::TopBottomPanel::bottom("rodape").show(ctx, |ui| self.rodape_configuracoes(ui));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.mostrar_aviso(ui);
            // Cada aba com a sua rolagem: sem isso, rolar uma rola todas.
            let id = match self.aba {
                Aba::Balanca => "rolagem-balanca",
                Aba::Configuracoes => "rolagem-config",
                Aba::Integracao => "rolagem-integracao",
                Aba::Log => "rolagem-log",
            };
            egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| match self.aba {
                Aba::Balanca => self.aba_balanca(ui, &estado),
                Aba::Configuracoes => self.aba_configuracoes(ui, &estado),
                Aba::Integracao => self.aba_integracao(ui),
                Aba::Log => self.aba_log(ui),
            });
        });
    }
}

// ----------------------------------------------------------------- auxiliares

fn cartao(ui: &mut Ui, corpo: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style()).inner_margin(12.0).rounding(8.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        corpo(ui);
    });
}

fn secao(ui: &mut Ui, titulo: &str, descricao: &str, corpo: impl FnOnce(&mut Ui)) {
    cartao(ui, |ui| {
        ui.label(RichText::new(titulo).size(16.0).strong());
        if !descricao.is_empty() {
            ui.label(RichText::new(descricao).weak());
        }
        ui.add_space(6.0);
        egui::Grid::new(titulo).num_columns(2).spacing([16.0, 10.0]).min_col_width(170.0).show(ui, corpo);
    });
    ui.add_space(10.0);
}

fn linha(ui: &mut Ui, rotulo: &str, dica: &str, campo: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(rotulo).strong());
    ui.vertical(|ui| {
        campo(ui);
        if !dica.is_empty() {
            ui.add(egui::Label::new(RichText::new(dica).small().weak()).wrap());
        }
    });
    ui.end_row();
}

fn linha_info(ui: &mut Ui, rotulo: &str, valor: &str) {
    ui.label(RichText::new(rotulo).weak());
    ui.label(RichText::new(valor).monospace());
    ui.end_row();
}

fn bolinha(ui: &mut Ui, cor: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 6.0, cor);
}

fn testar_protocolo(form: &Config, frame: &str) -> Result<String, String> {
    let bruto = interpretar_escapes(frame).map_err(|e| format!("Frame: {e}"))?;
    let protocolo = form.protocolo()?;
    let leitura = protocolo.interpretar(&bruto).map_err(|e| e.mensagem)?;
    let estavel = match leitura.estavel {
        Some(true) => "estável (pela regex)",
        Some(false) => "instável (pela regex)",
        None => "estabilidade calculada pelo agente",
    };
    Ok(format!("Peso: {} · {estavel}", formatar_peso(Some(leitura.peso), form.casas_decimais, &leitura.unidade)))
}

fn dica_do_erro(codigo: &str) -> Option<&'static str> {
    Some(match codigo {
        "PORTA_INDISPONIVEL" => "Porta errada, cabo solto ou outro programa usando a porta. Escolha a porta em Configurações > Porta serial.",
        "TIMEOUT" => "Balança desligada, baudrate errado, terminador errado, ou ela só responde depois do comando de peso.",
        "DADOS_INVALIDOS" => "A regex do peso não bate com o frame. Use Configurações > Testar protocolo com o último frame recebido.",
        "BALANCA_OFFLINE" => "A comunicação caiu. O agente tenta reconectar sozinho a cada leitura.",
        "COMANDO_NAO_SUPORTADO" => "Preencha os comandos de zero e tara em Configurações > Protocolo, conforme o manual da balança.",
        _ => return None,
    })
}

/// 1234.5 -> "1.234,50 kg"
pub fn formatar_peso(peso: Option<f64>, casas: u32, unidade: &str) -> String {
    let unidade = if unidade.is_empty() { "kg" } else { unidade };
    let Some(valor) = peso else {
        return format!("--,-- {unidade}");
    };
    let texto = format!("{:.*}", casas as usize, valor.abs());
    let (inteiro, decimal) = texto.split_once('.').unwrap_or((&texto, ""));
    let mut com_milhar = String::new();
    for (i, c) in inteiro.chars().enumerate() {
        if i > 0 && (inteiro.len() - i) % 3 == 0 {
            com_milhar.push('.');
        }
        com_milhar.push(c);
    }
    let sinal = if valor < 0.0 && texto.chars().any(|c| c.is_ascii_digit() && c != '0') { "-" } else { "" };
    if decimal.is_empty() {
        format!("{sinal}{com_milhar} {unidade}")
    } else {
        format!("{sinal}{com_milhar},{decimal} {unidade}")
    }
}

fn abrir(alvo: &str) {
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(alvo).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(alvo).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(alvo).spawn();
    if let Err(e) = r {
        warn!("Não foi possível abrir {alvo}: {e}");
    }
    let _ = Path::new(alvo);
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn formata_peso_em_portugues() {
        assert_eq!(formatar_peso(Some(1234.5), 2, "kg"), "1.234,50 kg");
        assert_eq!(formatar_peso(Some(0.0), 2, "kg"), "0,00 kg");
        assert_eq!(formatar_peso(Some(-12.3), 1, "kg"), "-12,3 kg");
        assert_eq!(formatar_peso(Some(1234567.0), 0, "g"), "1.234.567 g");
        assert_eq!(formatar_peso(None, 2, ""), "--,-- kg");
    }

    #[test]
    fn testa_protocolo_com_os_valores_do_formulario() {
        let cfg = Config { protocolo_regex_estavel: "^ST".into(), ..Config::default() };
        assert_eq!(testar_protocolo(&cfg, "ST,GS,+  125.40 kg\\r\\n").unwrap(), "Peso: 125,40 kg · estável (pela regex)");
        assert!(testar_protocolo(&cfg, "ERRO").is_err());
    }
}
