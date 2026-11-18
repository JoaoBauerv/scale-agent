//! Sobe e derruba o servidor HTTP. Ele roda numa thread própria, com o seu
//! runtime, para a janela não depender dele — e é reiniciado ao salvar as
//! configurações, porque porta, origens e token podem ter mudado.

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread::JoinHandle;

use log::{error, info};
use tokio::sync::oneshot;

use crate::api;
use crate::config::Config;
use crate::servico::ServicoPesagem;

pub struct ServidorApi {
    #[cfg_attr(not(test), allow(dead_code))]
    pub endereco: SocketAddr,
    encerrar: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl ServidorApi {
    /// O bind é feito aqui, antes da thread, para que "porta em uso" volte
    /// como erro para a janela mostrar, em vez de sumir num log.
    pub fn iniciar(cfg: &Config, servico: Arc<ServicoPesagem>) -> Result<Self, String> {
        let host = if cfg.host.trim() == "localhost" { "127.0.0.1" } else { cfg.host.trim() };
        let endereco: SocketAddr = format!("{host}:{}", cfg.porta_api)
            .parse()
            .map_err(|_| format!("Endereço inválido: {host}:{}", cfg.porta_api))?;

        let ouvinte = TcpListener::bind(endereco).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                format!(
                    "A porta {} já está em uso. Feche o outro programa (o agente Python, ou outra cópia deste) \
                     ou troque a porta em Configurações.",
                    cfg.porta_api
                )
            } else {
                format!("Não foi possível abrir a API em {endereco}: {e}")
            }
        })?;
        ouvinte.set_nonblocking(true).map_err(|e| e.to_string())?;
        // Com porta 0 o sistema escolhe uma livre; guarda a de verdade.
        let endereco = ouvinte.local_addr().unwrap_or(endereco);

        let app = api::rotas(servico, cfg);
        let (encerrar, sinal) = oneshot::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("api-http".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Não foi possível iniciar o servidor HTTP: {e}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let ouvinte = match tokio::net::TcpListener::from_std(ouvinte) {
                        Ok(o) => o,
                        Err(e) => {
                            error!("Não foi possível iniciar o servidor HTTP: {e}");
                            return;
                        }
                    };
                    let servidor = axum::serve(ouvinte, app.into_make_service_with_connect_info::<SocketAddr>())
                        .with_graceful_shutdown(async {
                            let _ = sinal.await;
                        });
                    if let Err(e) = servidor.await {
                        error!("Servidor HTTP parou: {e}");
                    }
                });
            })
            .map_err(|e| e.to_string())?;

        info!("API: http://{endereco}");
        Ok(Self { endereco, encerrar: Some(encerrar), thread: Some(thread) })
    }

    pub fn parar(&mut self) {
        if let Some(encerrar) = self.encerrar.take() {
            let _ = encerrar.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServidorApi {
    fn drop(&mut self) {
        self.parar();
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::servico::testes::config_rapida;

    #[test]
    fn sobe_responde_e_libera_a_porta() {
        let servico = ServicoPesagem::novo(config_rapida());
        servico.ciclo();
        // Porta 0: o sistema escolhe uma livre.
        let cfg = Config { porta_api: 0, ..config_rapida() };
        let mut servidor = ServidorApi::iniciar(&cfg, servico.clone()).unwrap();

        use std::io::{Read, Write};
        let mut conexao = std::net::TcpStream::connect(servidor.endereco).unwrap();
        conexao.write_all(b"GET /peso HTTP/1.1\r\nHost: teste\r\nConnection: close\r\n\r\n").unwrap();
        let mut resposta = String::new();
        conexao.read_to_string(&mut resposta).unwrap();
        assert!(resposta.starts_with("HTTP/1.1 200"), "{resposta}");
        assert!(resposta.contains("\"sucesso\":true"), "{resposta}");

        servidor.parar();
        assert!(TcpListener::bind(servidor.endereco).is_ok(), "a porta deve ficar livre depois de parar");
    }

    #[test]
    fn porta_em_uso_vira_mensagem_clara() {
        let ocupada = TcpListener::bind("127.0.0.1:0").unwrap();
        let porta = ocupada.local_addr().unwrap().port();
        let servico = ServicoPesagem::novo(config_rapida());
        let cfg = Config { porta_api: porta, ..config_rapida() };
        let erro = ServidorApi::iniciar(&cfg, servico).err().unwrap();
        assert!(erro.contains("já está em uso"), "{erro}");
    }
}
