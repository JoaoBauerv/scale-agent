# Scale Agent

Agente de balança para desktop, em Rust.

Programa independente que lê uma **balança** (física, pela porta serial/USB, ou
simulada) e entrega o peso por uma **API HTTP local**. Qualquer sistema web
pode usar: o navegador do computador onde a balança está ligado pede o peso ao
agente e o sistema recebe um JSON pronto.

```
BALANÇA ──USB/Serial──▶ SCALE AGENT ──HTTP/JSON──▶ NAVEGADOR ──▶ SEU SISTEMA WEB
(ou simulada)           127.0.0.1:5000
```

É um único `.exe`, sem instalar nada, com **janela própria**: visor ao vivo,
tela de Configurações com validação, dados de integração prontos para copiar e
log.

---

## 1. Usar

Baixe o `ScaleAgent-portatil.zip` na seção
[*Releases*](https://github.com/JoaoBauerv/scale-agent/releases) do GitHub (ou
compile, seção 9), extraia e abra o `scale-agent.exe`. Na primeira vez ele usa
a configuração padrão: balança **simulada** e API em `http://127.0.0.1:5000`.

> O Windows pode mostrar "O Windows protegeu o computador", porque o `.exe`
> não tem assinatura digital: clique em **Mais informações → Executar assim
> mesmo**.

A janela tem quatro abas:

| Aba | Para que serve |
|---|---|
| **Balança** | Visor ao vivo (verde = estável, amarelo = estabilizando), Gerar peso (simulada), Aplicar tara, Zerar, alternar simulada ⇄ real |
| **Configurações** | Todos os parâmetros, agrupados por assunto. **Salvar e aplicar** valida, grava e aplica na hora, sem reiniciar |
| **Integração** | Endereço, rotas e campos que o sistema web usa, com botão de copiar, e a resposta atual de `GET /peso` |
| **Log** | Últimas mensagens; o arquivo completo fica em `logs/scale-agent.log` |

Deixe o agente aberto enquanto o sistema estiver em uso. Marque
**Configurações → Log e Windows → Iniciar com o Windows** para ele abrir
sozinho, minimizado, ao ligar o computador.

## 2. Integrar com um sistema web

O sistema web chama o agente a partir do navegador do computador da balança:

```js
const resposta = await fetch("http://127.0.0.1:5000/peso");
const dados = await resposta.json();
if (dados.sucesso && dados.estavel) {
  campoPeso.value = dados.peso;          // ex.: 125.4
} else if (!dados.sucesso) {
  mostrarErro(dados.erro);               // ex.: "Balança offline"
}

// opcional: registrar no log do agente que o peso foi usado
await fetch("http://127.0.0.1:5000/captura", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ peso: dados.peso, origem: "tela de pesagem" }),
});
```

| O que o sistema precisa saber | Valor padrão |
|---|---|
| Endereço do agente | `http://127.0.0.1:5000` |
| Rota de leitura do peso | `GET /peso` |
| Rota de registro da captura | `POST /captura` |
| Campos da resposta | `sucesso`, `peso`, `unidade`, `estavel`, `erro`, `codigo`, `modo` |

A aba **Integração** mostra esses valores com a porta que estiver configurada.
Todas as rotas estão na seção 6.

**Sistema publicado na internet** (ex.: Vercel): o endereço do sistema precisa
estar em **Configurações → API local → Origens permitidas** (ex.:
`https://meusistema.vercel.app`), senão o navegador bloqueia a chamada
(CORS). `localhost` e `127.0.0.1` já vêm liberados. O agente também responde à
checagem de *Private Network Access* do Chrome, que bloqueia sites da internet
de chamar `127.0.0.1` sem permissão explícita.

## 3. Configurações

Ficam em `config.toml`, nesta ordem de procura:

1. ao lado do `.exe`, se existir um `config.toml` lá (**modo portátil**: leve a
   pasta num pendrive — é assim que vem o zip da Release);
2. senão, `%APPDATA%\ScaleAgent\config.toml`.

Os logs ficam na pasta `logs` do mesmo lugar. O botão **Abrir pasta** da aba
Configurações abre esse diretório.

| Chave (`config.toml`) | Padrão | O que é |
|---|---|---|
| `modo_balanca` | `simulada` | `simulada` ou `real` ao abrir |
| `unidade` | `kg` | |
| `host` / `porta_api` | `127.0.0.1` / `5000` | onde a API sobe |
| `origens_permitidas` | `http://localhost,http://127.0.0.1` | CORS; vírgula ou uma por linha; `*` libera tudo |
| `permitir_rede` | `false` | `false` = só aceita chamadas deste computador |
| `api_token` | vazio | protege `/zero`, `/tara`, `/modo` e `/simulada/gerar` (header `X-API-Token`) |
| `intervalo_atualizacao` | `0.5` | segundos entre leituras |
| `tolerancia_estabilidade` | `0.05` | variação máxima para contar como parado |
| `tempo_estabilidade` | `1.5` | segundos parado para ser estável |
| `casas_decimais` | `2` | |
| `sim_peso_minimo` / `sim_peso_maximo` | `0` / `200` | faixa do Gerar peso |
| `sim_peso_inicial` | `0` | |
| `sim_resolucao` | `0.10` | divisão da balança simulada |
| `porta_com` | `COM3` | |
| `baudrate` / `bytesize` | `9600` / `8` | |
| `paridade` | `N` | `N`, `E` ou `O` |
| `stopbits` | `1` | `1` ou `2` |
| `timeout` | `1.0` | segundos esperando um frame completo |
| `comando_peso` | vazio | o que enviar para pedir o peso; vazio = balança transmite sozinha |
| `terminador` | `\r\n` | fim de cada frame |
| `protocolo_regex` | `([-+]?\d+[.,]?\d*)` | o grupo 1 (ou `(?P<peso>...)`) é o peso |
| `protocolo_regex_estavel` | vazio | ex.: `^ST`; vazio = o agente calcula |
| `fator_divisao` | `1` | `012540` com fator `100` = 125,40 |
| `comando_zero` / `comando_tara` | vazio | vazio = `COMANDO_NAO_SUPORTADO` |
| `log_nivel` | `INFO` | `DEBUG`, `INFO`, `WARNING`, `ERROR` |
| `iniciar_com_windows` | `false` | grava/remove o agente em `HKCU\...\Run` |

Comandos e terminador aceitam os escapes `\r`, `\n`, `\t`, `\0`, `\\` e `\xHH`.

Editar o `config.toml` à mão funciona, mas vale só ao reabrir o agente, e o
que estiver inválido cai no padrão com aviso no log. Pela janela, nada é salvo
sem passar pela validação.

## 4. Balança simulada

Sem equipamento nenhum. **Gerar peso** sorteia um valor na faixa configurada
(ou use **Colocar este peso** para um valor específico); o peso vai direto
para o visor, respeitando a resolução (com `0,10`, 195,47 vira 195,50), e vira
**estável** depois do *tempo parado*. Tara e zero funcionam como numa balança
de verdade.

Pela API:

```bash
curl -X POST http://127.0.0.1:5000/simulada/gerar -H "Content-Type: application/json" -d "{\"peso\": 125.40}"
```

## 5. Balança física

1. Conecte a balança e escolha a porta em **Configurações → Porta serial**
   (o botão *Escolher...* lista as portas da máquina);
2. Preencha baudrate, bits, paridade e stop bits conforme o **manual do
   equipamento**;
3. Em **Protocolo**, ajuste terminador, regex e, se a balança precisar,
   comando de peso, zero e tara;
4. Em **Operação**, escolha *Real* e clique em **Salvar e aplicar**.

O protocolo **não está chutado no código**: cada fabricante manda os dados de
um jeito. Para montar a regex, veja o frame cru em **Balança → Conexão →
Último frame recebido** e use **Configurações → Protocolo → Testar protocolo**:
cole o frame (ou clique *Usar o último recebido*) e veja o peso que o agente
entende com os valores do formulário, antes de salvar.

Exemplo — balança que envia `ST,GS,+  125.40 kg\r\n`:

| Campo | Valor |
|---|---|
| Comando de peso | *(vazio)* |
| Terminador | `\r\n` |
| Regex do peso | `([-+]?\d+[.,]?\d*)` |
| Regex de estabilidade | `^ST` |
| Fator de divisão | `1` |

**Leitura contínua:** quando a balança transmite sozinha, os frames se
acumulam na porta entre uma leitura e outra. O agente lê tudo o que chegou e
usa o **último** frame completo (assim o visor nunca fica atrasado), e descarta o pedaço de frame que estava no meio do caminho quando a
porta foi aberta.

**Regex:** é a sintaxe do crate `regex` do Rust, que cobre tudo o que um
frame de balança precisa (grupos, `(?P<nome>...)`, classes, âncoras), mas sem *lookaround* (`(?=...)`, `(?<=...)`) nem referências para trás (`\1`).

## 6. API

| Rota | Método | O que faz |
|---|---|---|
| `/` | GET | Página de acompanhamento no navegador |
| `/peso` | GET | Peso atual, unidade, estabilidade e modo |
| `/status` | GET | Situação da conexão, parâmetros e último frame |
| `/portas` | GET | Portas seriais da máquina |
| `/zero` | POST | Zera a balança |
| `/tara` | POST | Aplica tara |
| `/modo` | POST | Alterna simulada ⇄ real — `{"modo": "real"}` |
| `/simulada/gerar` | POST | Novo peso na simulação — `{"peso": 137.8}` ou sem corpo para sortear |
| `/captura` | POST | Registra no log que o sistema capturou o peso — `{"peso": 12.5, "origem": "tela de pesagem"}` |

```json
{ "sucesso": true, "peso": 125.40, "unidade": "kg", "estavel": true, "modo": "real", "erro": null, "codigo": null }
```

Quando algo dá errado, o formato é o mesmo, com `"sucesso": false`, a mensagem
em `erro` e o código em `codigo`: `BALANCA_OFFLINE`, `PORTA_INDISPONIVEL`,
`TIMEOUT`, `DADOS_INVALIDOS`, `COMANDO_NAO_SUPORTADO` ou `ERRO_BALANCA`.

Respostas HTTP de recusa: **403** fora da máquina com *Permitir rede*
desligado, **401** sem o token nas rotas protegidas, **400** em modo inválido
ou Gerar peso fora do modo simulado.

## 7. Diagnosticar

A aba **Balança** mostra o código do erro e o que costuma causá-lo.

| Sintoma | Provável causa |
|---|---|
| Painel não aparece no sistema web | Agente fechado, endereço errado no sistema, ou origem do sistema fora de *Origens permitidas* |
| "API parada" no topo da janela | Porta em uso (outra cópia do agente ou outro programa aberto) — troque a porta ou feche o outro |
| `PORTA_INDISPONIVEL` | Porta errada, cabo solto, ou outro programa usando a porta |
| `TIMEOUT` | Balança desligada, baudrate ou terminador errado, ou ela só responde depois do comando de peso |
| `DADOS_INVALIDOS` | A regex não bate com o frame — use *Testar protocolo* |
| Peso nunca fica estável | Aumente a tolerância ou diminua o tempo parado |
| Peso multiplicado/dividido errado | Ajuste o fator de divisão |

Para registrar cada leitura, use o nível de log `DEBUG`.

## 8. Segurança

- A API sobe em `127.0.0.1`: não fica exposta na rede;
- Com *Permitir rede* desligado (padrão), chamadas de fora da máquina recebem 403;
- O token, se preenchido, protege as rotas que **alteram** a balança;
- O CORS só libera `localhost`, `127.0.0.1` e as origens configuradas.

## 9. Compilar

Requer o Rust (<https://rustup.rs>), versão 1.80 ou superior.

```bash
cargo run                 # versão de desenvolvimento, com console mostrando o log
cargo test                # 44 testes, todos sem hardware
cargo build --release     # target/release/scale-agent.exe (~8 MB, sem console)
```

Parâmetros de linha de comando:

| Parâmetro | Efeito |
|---|---|
| `--minimizado` | abre minimizado (usado pelo *Iniciar com o Windows*) |
| `--sem-janela` | só a API, sem janela |
| `--aba=configuracoes` | abre direto numa aba (`configuracoes`, `integracao`, `log`) |

Versões novas são compiladas pelo GitHub Actions
(`.github/workflows/release.yml`): ao enviar uma tag `v*`, ele roda os testes,
compila no Windows e publica a Release com o zip portátil (`.exe` +
`pacote/config.toml`).

```bash
git tag v1.0.1
git push origin v1.0.1
```

## 10. Estrutura

```
src/
├── main.rs            parâmetros, log, sobe serviço, API e janela
├── config.rs          config.toml: campos, padrões, validação, escapes
├── servico.rs         leitura periódica, estabilidade, troca de modo, reconexão
├── api.rs             rotas HTTP, CORS, bloqueio de rede e token
├── servidor.rs        sobe/derruba a API (reiniciada ao salvar as configurações)
├── gui.rs             janela: Balança, Configurações, Integração, Log
├── registro.rs        log em arquivo, console e memória (aba Log)
├── autostart.rs       Iniciar com o Windows (registro HKCU)
├── erro.rs            códigos de erro da API
└── balanca/
    ├── mod.rs         contrato comum (trait Balanca)
    ├── simulada.rs    balança sem hardware
    └── serial.rs      porta COM e protocolo configurável
assets/index.html      página de acompanhamento servida em /
assets/icon.ico/.png   ícone do executável e da janela
pacote/config.toml     configuração que vai no zip da Release
```
