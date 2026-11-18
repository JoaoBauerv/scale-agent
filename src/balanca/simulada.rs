//! Balança simulada — funciona sem nenhum hardware conectado.
//!
//! O peso vai direto para o valor gerado (sem rampa) e fica parado ali, como
//! uma balança que já acomodou a carga. Quem decide se está "estável" é o
//! serviço de pesagem, pela mesma regra usada com a balança física.
//!
//! Os valores respeitam a resolução (divisão) da balança: com 0,10 kg o visor
//! mostra 195,50 — nunca 195,47.

use rand::Rng;
use serde_json::json;

use super::{arredondar, Balanca, LeituraPeso};
use crate::erro::ErroBalanca;

pub struct BalancaSimulada {
    pub peso_minimo: f64,
    pub peso_maximo: f64,
    pub resolucao: f64,
    pub unidade: String,
    pub casas_decimais: u32,
    conectada: bool,
    peso_bruto: f64,
    tara: f64,
}

impl BalancaSimulada {
    pub fn nova(
        peso_minimo: f64,
        peso_maximo: f64,
        peso_inicial: f64,
        resolucao: f64,
        unidade: &str,
        casas_decimais: u32,
    ) -> Self {
        let mut balanca = Self {
            peso_minimo,
            peso_maximo,
            resolucao: if resolucao > 0.0 { resolucao } else { 0.10 },
            unidade: unidade.to_string(),
            casas_decimais,
            conectada: false,
            peso_bruto: 0.0,
            tara: 0.0,
        };
        balanca.peso_bruto = balanca.ajustar(peso_inicial);
        balanca
    }

    /// Simula material sendo colocado na balança. Sem valor, sorteia na faixa.
    pub fn gerar_peso(&mut self, valor: Option<f64>) -> f64 {
        let valor = valor.unwrap_or_else(|| {
            if self.peso_maximo > self.peso_minimo {
                rand::thread_rng().gen_range(self.peso_minimo..=self.peso_maximo)
            } else {
                self.peso_minimo
            }
        });
        self.peso_bruto = self.ajustar(valor + self.tara);
        self.ajustar(self.peso_bruto - self.tara)
    }

    pub fn tara_aplicada(&self) -> f64 {
        arredondar(self.tara, self.casas_decimais)
    }

    /// Encaixa o valor na resolução da balança e respeita os limites.
    fn ajustar(&self, valor: f64) -> f64 {
        let passos = (valor / self.resolucao).round();
        let ajustado = arredondar(passos * self.resolucao, self.casas_decimais);
        ajustado.clamp(self.peso_minimo, self.peso_maximo.max(self.peso_minimo))
    }
}

impl Default for BalancaSimulada {
    fn default() -> Self {
        Self::nova(0.0, 200.0, 0.0, 0.10, "kg", 2)
    }
}

impl Balanca for BalancaSimulada {
    fn modo(&self) -> &'static str {
        "simulado"
    }

    fn conectar(&mut self) -> Result<(), ErroBalanca> {
        self.conectada = true;
        Ok(())
    }

    fn desconectar(&mut self) {
        self.conectada = false;
    }

    fn esta_conectada(&self) -> bool {
        self.conectada
    }

    fn ler_peso(&mut self) -> Result<LeituraPeso, ErroBalanca> {
        // `estavel: None` de propósito: quem decide é o serviço de pesagem.
        Ok(LeituraPeso {
            peso: self.ajustar(self.peso_bruto - self.tara),
            unidade: self.unidade.clone(),
            estavel: None,
        })
    }

    fn zerar(&mut self) -> Result<(), ErroBalanca> {
        self.peso_bruto = 0.0;
        self.tara = 0.0;
        Ok(())
    }

    fn tarar(&mut self) -> Result<(), ErroBalanca> {
        self.tara = self.peso_bruto;
        Ok(())
    }

    fn descricao(&self) -> serde_json::Value {
        json!({
            "modo": self.modo(),
            "peso_minimo": self.peso_minimo,
            "peso_maximo": self.peso_maximo,
            "resolucao": self.resolucao,
            "tara_aplicada": self.tara_aplicada(),
        })
    }

    fn como_simulada(&mut self) -> Option<&mut BalancaSimulada> {
        Some(self)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn perto(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn conectada(min: f64, max: f64, res: f64) -> BalancaSimulada {
        let mut b = BalancaSimulada::nova(min, max, 0.0, res, "kg", 2);
        b.conectar().unwrap();
        b
    }

    #[test]
    fn vai_direto_para_o_peso_gerado() {
        let mut b = conectada(0.0, 200.0, 0.10);
        assert!(perto(b.ler_peso().unwrap().peso, 0.0, 0.01));
        b.gerar_peso(Some(125.40));
        assert!(perto(b.ler_peso().unwrap().peso, 125.40, 0.01));
    }

    #[test]
    fn mantem_o_peso_parado() {
        let mut b = conectada(0.0, 200.0, 0.10);
        b.gerar_peso(Some(23.40));
        let leituras: Vec<f64> = (0..3).map(|_| b.ler_peso().unwrap().peso).collect();
        assert_eq!(leituras, vec![23.40, 23.40, 23.40]);
    }

    #[test]
    fn peso_aleatorio_respeita_a_faixa() {
        let mut b = conectada(10.0, 200.0, 0.10);
        for _ in 0..50 {
            let p = b.gerar_peso(None);
            assert!((10.0..=200.0).contains(&p), "peso {p} fora da faixa");
        }
    }

    #[test]
    fn respeita_a_resolucao() {
        let mut b = conectada(0.0, 200.0, 0.10);
        assert!(perto(b.gerar_peso(Some(195.47)), 195.50, 1e-9));
        assert!(perto(b.gerar_peso(Some(11.23)), 11.20, 1e-9));
        for _ in 0..50 {
            let p = b.gerar_peso(None);
            let centavos = (p * 100.0).round() as i64;
            assert_eq!(centavos % 10, 0, "peso {p:.2} não é múltiplo de 0,10");
            assert!(perto(b.ler_peso().unwrap().peso, p, 1e-9));
        }
    }

    #[test]
    fn resolucao_configuravel() {
        let mut b = conectada(0.0, 200.0, 0.5);
        assert!(perto(b.gerar_peso(Some(195.47)), 195.50, 1e-9));
        assert!(perto(b.gerar_peso(Some(195.20)), 195.00, 1e-9));
    }

    #[test]
    fn zerar_e_tarar() {
        let mut b = conectada(0.0, 200.0, 0.10);
        b.gerar_peso(Some(125.0));
        assert!(perto(b.ler_peso().unwrap().peso, 125.0, 0.01));
        b.tarar().unwrap();
        assert!(perto(b.ler_peso().unwrap().peso, 0.0, 0.01));
        b.gerar_peso(Some(30.0));
        assert!(perto(b.ler_peso().unwrap().peso, 30.0, 0.01));
        b.zerar().unwrap();
        assert!(perto(b.ler_peso().unwrap().peso, 0.0, 0.01));
    }

    #[test]
    fn respeita_limites() {
        let mut b = conectada(0.0, 50.0, 0.10);
        b.gerar_peso(Some(9999.0));
        assert!(b.ler_peso().unwrap().peso <= 50.0);
    }
}
