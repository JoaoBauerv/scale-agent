//! Rotas HTTP consumidas pelo sistema web e pela página de acompanhamento.
//!
//! O sistema web lê `sucesso`, `peso`, `unidade`, `estavel` e `erro` em
//! GET /peso e pode avisar em POST /captura quando usar o peso.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use log::warn;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::balanca::serial::listar_portas;
use crate::config::Config;
use crate::erro::ErroBalanca;
use crate::servico::ServicoPesagem;

const PAGINA: &str = include_str!("../assets/index.html");

#[derive(Clone)]
pub struct EstadoApi {
    pub servico: Arc<ServicoPesagem>,
    pub permitir_rede: bool,
    pub api_token: String,
}

pub fn rotas(servico: Arc<ServicoPesagem>, cfg: &Config) -> Router {
    let estado = EstadoApi { servico, permitir_rede: cfg.permitir_rede, api_token: cfg.api_token.clone() };

    Router::new()
        .route("/", get(pagina))
        .route("/peso", get(peso))
        .route("/status", get(status))
        .route("/portas", get(portas))
        .route("/zero", post(zero))
        .route("/tara", post(tara))
        .route("/modo", post(modo))
        .route("/simulada/gerar", post(gerar))
        .route("/captura", post(captura))
        .layer(cors(cfg))
        .with_state(estado)
}

/// CORS: libera as origens configuradas e sempre localhost/127.0.0.1.
///
/// `allow_private_network` responde à checagem do Chrome para páginas na
/// internet (ex.: o sistema publicado na Vercel) que chamam um endereço
/// local como 127.0.0.1. Sem ela, o navegador bloqueia a leitura do peso.
fn cors(cfg: &Config) -> CorsLayer {
    let origens = cfg.origens();
    let origem = if origens.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        AllowOrigin::predicate(move |origem: &HeaderValue, _| {
            let Ok(texto) = origem.to_str() else { return false };
            origem_local(texto) || origens.iter().any(|o| o == texto)
        })
    };

    CorsLayer::new()
        .allow_origin(origem)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::HeaderName::from_static("x-api-token")])
        .allow_private_network(true)
}

fn origem_local(origem: &str) -> bool {
    let resto = origem.strip_prefix("http://").or_else(|| origem.strip_prefix("https://"));
    let Some(resto) = resto else { return false };
    let host = resto.split(':').next().unwrap_or("");
    let porta_ok = resto.split_once(':').map_or(true, |(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    (host == "localhost" || host == "127.0.0.1") && porta_ok
}

// ------------------------------------------------------------------ segurança

fn negar(status: StatusCode, detalhe: &str) -> Response {
    (status, Json(json!({ "detail": detalhe }))).into_response()
}

#[allow(clippy::result_large_err)]
/// Bloqueia chamadas de fora da máquina quando "Permitir rede" está desligado.
fn exigir_local(estado: &EstadoApi, endereco: SocketAddr) -> Result<(), Response> {
    if estado.permitir_rede || endereco.ip().is_loopback() {
        return Ok(());
    }
    warn!("Requisição bloqueada de {} (permitir rede desligado)", endereco.ip());
    Err(negar(StatusCode::FORBIDDEN, "Acesso permitido apenas localmente"))
}

#[allow(clippy::result_large_err)]
/// Protege as rotas que ALTERAM o estado da balança.
fn exigir_autorizacao(estado: &EstadoApi, endereco: SocketAddr, headers: &HeaderMap, rota: &str) -> Result<(), Response> {
    exigir_local(estado, endereco)?;
    if estado.api_token.is_empty() {
        return Ok(());
    }
    let enviado = headers.get("x-api-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if enviado != estado.api_token {
        warn!("Token inválido em {rota}");
        return Err(negar(StatusCode::UNAUTHORIZED, "Token de API inválido ou ausente"));
    }
    Ok(())
}

fn erro_balanca(e: ErroBalanca) -> Response {
    Json(json!({ "sucesso": false, "erro": e.mensagem, "codigo": e.codigo.como_str() })).into_response()
}

/// Comandos podem esperar a porta serial; saem da thread do servidor.
async fn em_segundo_plano<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("comando da balança falhou")
}

// ------------------------------------------------------------------ consultas

async fn pagina() -> Html<&'static str> {
    Html(PAGINA)
}

async fn peso(State(estado): State<EstadoApi>, ConnectInfo(endereco): ConnectInfo<SocketAddr>) -> Response {
    if let Err(r) = exigir_local(&estado, endereco) {
        return r;
    }
    let e = estado.servico.estado();

    if !e.conectada || e.peso.is_none() {
        return Json(json!({
            "sucesso": false,
            "peso": null,
            "unidade": null,
            "estavel": null,
            "modo": e.modo,
            "erro": e.erro.unwrap_or_else(|| "Balança não conectada".into()),
            "codigo": e.codigo_erro.unwrap_or_else(|| "BALANCA_OFFLINE".into()),
        }))
        .into_response();
    }

    Json(json!({
        "sucesso": true,
        "peso": e.peso,
        "unidade": e.unidade,
        "estavel": e.estavel,
        "modo": e.modo,
        "erro": null,
        "codigo": null,
    }))
    .into_response()
}

async fn status(State(estado): State<EstadoApi>, ConnectInfo(endereco): ConnectInfo<SocketAddr>) -> Response {
    if let Err(r) = exigir_local(&estado, endereco) {
        return r;
    }
    let e = estado.servico.estado();
    let cfg = estado.servico.config();
    Json(json!({
        "conectada": e.conectada,
        "modo": e.modo,
        "unidade": e.unidade,
        "estavel": e.estavel,
        "peso": e.peso,
        "erro": e.erro,
        "codigo": e.codigo_erro,
        "atualizado_em": e.atualizado_em,
        "conexao": e.conexao,
        "ultimo_frame": e.ultimo_frame,
        "estabilidade": { "tolerancia": cfg.tolerancia_estabilidade, "tempo": cfg.tempo_estabilidade },
        "agente": { "versao": env!("CARGO_PKG_VERSION"), "implementacao": "rust" },
    }))
    .into_response()
}

async fn portas(State(estado): State<EstadoApi>, ConnectInfo(endereco): ConnectInfo<SocketAddr>) -> Response {
    if let Err(r) = exigir_local(&estado, endereco) {
        return r;
    }
    let lista = em_segundo_plano(listar_portas).await;
    Json(json!({ "sucesso": true, "portas": lista })).into_response()
}

// ------------------------------------------------------------------- comandos

async fn zero(State(estado): State<EstadoApi>, ConnectInfo(endereco): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if let Err(r) = exigir_autorizacao(&estado, endereco, &headers, "/zero") {
        return r;
    }
    let servico = estado.servico.clone();
    match em_segundo_plano(move || servico.zerar()).await {
        Ok(()) => Json(json!({ "sucesso": true, "mensagem": "Balança zerada" })).into_response(),
        Err(e) => erro_balanca(e),
    }
}

async fn tara(State(estado): State<EstadoApi>, ConnectInfo(endereco): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if let Err(r) = exigir_autorizacao(&estado, endereco, &headers, "/tara") {
        return r;
    }
    let servico = estado.servico.clone();
    match em_segundo_plano(move || servico.tarar()).await {
        Ok(()) => Json(json!({ "sucesso": true, "mensagem": "Tara aplicada" })).into_response(),
        Err(e) => erro_balanca(e),
    }
}

#[derive(Deserialize)]
struct CorpoModo {
    modo: String,
}

async fn modo(
    State(estado): State<EstadoApi>,
    ConnectInfo(endereco): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    corpo: Option<Json<CorpoModo>>,
) -> Response {
    if let Err(r) = exigir_autorizacao(&estado, endereco, &headers, "/modo") {
        return r;
    }
    let Some(Json(corpo)) = corpo else {
        return negar(StatusCode::BAD_REQUEST, "Envie {\"modo\": \"simulada\"} ou {\"modo\": \"real\"}");
    };
    let servico = estado.servico.clone();
    match em_segundo_plano(move || servico.definir_modo(&corpo.modo)).await {
        Ok(e) => Json(json!({ "sucesso": true, "modo": e.modo, "conectada": e.conectada })).into_response(),
        Err(msg) => negar(StatusCode::BAD_REQUEST, &msg),
    }
}

#[derive(Deserialize, Default)]
struct CorpoGerar {
    peso: Option<f64>,
}

async fn gerar(
    State(estado): State<EstadoApi>,
    ConnectInfo(endereco): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    corpo: Option<Json<CorpoGerar>>,
) -> Response {
    if let Err(r) = exigir_autorizacao(&estado, endereco, &headers, "/simulada/gerar") {
        return r;
    }
    let valor = corpo.and_then(|Json(c)| c.peso);
    let servico = estado.servico.clone();
    match em_segundo_plano(move || servico.gerar_peso(valor)).await {
        Ok(alvo) => Json(json!({ "sucesso": true, "peso_alvo": alvo })).into_response(),
        Err(msg) => negar(StatusCode::BAD_REQUEST, &msg),
    }
}

#[derive(Deserialize)]
struct CorpoCaptura {
    peso: f64,
    #[serde(default = "origem_padrao")]
    origem: String,
}

fn origem_padrao() -> String {
    "sistema".into()
}

async fn captura(
    State(estado): State<EstadoApi>,
    ConnectInfo(endereco): ConnectInfo<SocketAddr>,
    corpo: Result<Json<CorpoCaptura>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Err(r) = exigir_local(&estado, endereco) {
        return r;
    }
    match corpo {
        Ok(Json(c)) => {
            estado.servico.registrar_captura(c.peso, &c.origem);
            Json(json!({ "sucesso": true })).into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "detail": e.body_text() }))).into_response(),
    }
}

/// Exemplo do que GET /peso devolve agora — mostrado na aba Integração.
pub fn exemplo_resposta_peso(servico: &ServicoPesagem) -> Value {
    let e = servico.estado();
    if e.conectada && e.peso.is_some() {
        json!({ "sucesso": true, "peso": e.peso, "unidade": e.unidade, "estavel": e.estavel, "modo": e.modo })
    } else {
        json!({ "sucesso": false, "erro": e.erro, "codigo": e.codigo_erro, "modo": e.modo })
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::servico::testes::config_rapida;
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app(cfg: Config, de: [u8; 4]) -> (Router, Arc<ServicoPesagem>) {
        let servico = ServicoPesagem::novo(cfg.clone());
        servico.ciclo();
        let router = rotas(servico.clone(), &cfg).layer(MockConnectInfo(SocketAddr::from((de, 40000))));
        (router, servico)
    }

    fn local(cfg: Config) -> (Router, Arc<ServicoPesagem>) {
        app(cfg, [127, 0, 0, 1])
    }

    async fn chamar(router: &Router, metodo: &str, rota: &str, corpo: Option<Value>, token: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(metodo).uri(rota);
        if let Some(t) = token {
            req = req.header("x-api-token", t);
        }
        let req = match corpo {
            Some(c) => req.header("content-type", "application/json").body(Body::from(c.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let resposta = router.clone().oneshot(req).await.unwrap();
        let status = resposta.status();
        let bytes = resposta.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn peso_no_modo_simulado() {
        let (r, _) = local(config_rapida());
        let (status, dados) = chamar(&r, "GET", "/peso", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(dados["sucesso"], true);
        assert_eq!(dados["modo"], "simulado");
        assert_eq!(dados["unidade"], "kg");
        assert!(dados["peso"].is_f64());
        assert!(dados["estavel"].is_boolean());
    }

    #[tokio::test]
    async fn status_da_balanca() {
        let (r, _) = local(config_rapida());
        let (_, dados) = chamar(&r, "GET", "/status", None, None).await;
        assert_eq!(dados["conectada"], true);
        assert_eq!(dados["modo"], "simulado");
        assert_eq!(dados["unidade"], "kg");
    }

    #[tokio::test]
    async fn gerar_peso_reflete_na_leitura() {
        let (r, _) = local(config_rapida());
        let (_, dados) = chamar(&r, "POST", "/simulada/gerar", Some(json!({ "peso": 137.80 })), None).await;
        assert_eq!(dados["sucesso"], true);
        let (_, dados) = chamar(&r, "GET", "/peso", None, None).await;
        assert_eq!(dados["peso"], 137.8);

        // Sem corpo, sorteia — como o botão da página.
        let (status, dados) = chamar(&r, "POST", "/simulada/gerar", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(dados["peso_alvo"].is_f64());
    }

    #[tokio::test]
    async fn zero_e_tara() {
        let (r, _) = local(config_rapida());
        chamar(&r, "POST", "/simulada/gerar", Some(json!({ "peso": 80 })), None).await;
        assert_eq!(chamar(&r, "POST", "/tara", Some(json!({})), None).await.1["sucesso"], true);
        assert_eq!(chamar(&r, "POST", "/zero", Some(json!({})), None).await.1["sucesso"], true);
        let (_, dados) = chamar(&r, "GET", "/peso", None, None).await;
        assert_eq!(dados["peso"], 0.0);
    }

    #[tokio::test]
    async fn erro_quando_balanca_real_nao_conecta() {
        let (r, _) = local(config_rapida());
        chamar(&r, "POST", "/modo", Some(json!({ "modo": "real" })), None).await;
        let (_, dados) = chamar(&r, "GET", "/peso", None, None).await;
        assert_eq!(dados["sucesso"], false);
        assert_eq!(dados["codigo"], "PORTA_INDISPONIVEL");
        assert!(dados["erro"].as_str().unwrap().contains("COM_TESTE_INEXISTENTE"));

        // Tara sem comando configurado na balança real.
        let (_, dados) = chamar(&r, "POST", "/tara", Some(json!({})), None).await;
        assert_eq!(dados["sucesso"], false);
    }

    #[tokio::test]
    async fn modo_invalido() {
        let (r, _) = local(config_rapida());
        assert_eq!(chamar(&r, "POST", "/modo", Some(json!({ "modo": "xpto" })), None).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn captura_registra_e_valida_o_corpo() {
        let (r, _) = local(config_rapida());
        let (status, dados) = chamar(&r, "POST", "/captura", Some(json!({ "peso": 12.5, "origem": "tela de compra" })), None).await;
        assert_eq!((status, dados["sucesso"].clone()), (StatusCode::OK, json!(true)));
        assert_eq!(chamar(&r, "POST", "/captura", Some(json!({ "origem": "x" })), None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn bloqueia_origem_externa() {
        let (r, _) = app(Config { permitir_rede: false, ..config_rapida() }, [192, 168, 0, 10]);
        assert_eq!(chamar(&r, "GET", "/peso", None, None).await.0, StatusCode::FORBIDDEN);
        assert_eq!(chamar(&r, "POST", "/zero", Some(json!({})), None).await.0, StatusCode::FORBIDDEN);

        let (r, _) = app(Config { permitir_rede: true, ..config_rapida() }, [192, 168, 0, 10]);
        assert_eq!(chamar(&r, "GET", "/peso", None, None).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn exige_token_quando_configurado() {
        let (r, _) = local(Config { api_token: "segredo".into(), ..config_rapida() });
        assert_eq!(chamar(&r, "GET", "/peso", None, None).await.0, StatusCode::OK, "leitura não exige token");
        assert_eq!(chamar(&r, "POST", "/zero", Some(json!({})), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(chamar(&r, "POST", "/zero", Some(json!({})), Some("errado")).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(chamar(&r, "POST", "/zero", Some(json!({})), Some("segredo")).await.0, StatusCode::OK);
        assert_eq!(chamar(&r, "POST", "/captura", Some(json!({ "peso": 1 })), None).await.0, StatusCode::OK, "captura não exige token");
    }

    async fn preflight(router: &Router, origem: &str) -> axum::http::HeaderMap {
        let req = Request::builder()
            .method("OPTIONS")
            .uri("/peso")
            .header("origin", origem)
            .header("access-control-request-method", "GET")
            .header("access-control-request-private-network", "true")
            .body(Body::empty())
            .unwrap();
        router.clone().oneshot(req).await.unwrap().headers().clone()
    }

    #[tokio::test]
    async fn cors_libera_localhost_as_origens_e_rede_privada() {
        let (r, _) = local(Config { origens_permitidas: "https://meusistema.vercel.app".into(), ..config_rapida() });

        let h = preflight(&r, "https://meusistema.vercel.app").await;
        assert_eq!(h["access-control-allow-origin"], "https://meusistema.vercel.app");
        assert_eq!(h["access-control-allow-private-network"], "true");

        assert_eq!(preflight(&r, "http://localhost:8080").await["access-control-allow-origin"], "http://localhost:8080");
        assert!(preflight(&r, "https://outro.com").await.get("access-control-allow-origin").is_none());
        assert!(preflight(&r, "http://localhost.evil.com").await.get("access-control-allow-origin").is_none());
    }

    #[test]
    fn reconhece_origem_local() {
        assert!(origem_local("http://localhost"));
        assert!(origem_local("http://127.0.0.1:5000"));
        assert!(!origem_local("http://localhost.evil.com"));
        assert!(!origem_local("http://127.0.0.1:abc"));
        assert!(!origem_local("ftp://localhost"));
    }
}
