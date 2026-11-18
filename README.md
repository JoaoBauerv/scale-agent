# Agente de Balança (desktop, Rust)

Programa de desktop que fica entre a **balança** (física ou simulada) e o
**sistema web** de gestão de reciclagem.

```
BALANÇA FÍSICA ──USB/Serial──▶ AGENTE (este programa) ──HTTP/JSON──▶ NAVEGADOR DO BALCÃO ──▶ SISTEMA WEB
                               (ou simulada)            127.0.0.1:5000                       (tela de compra/venda)
```

É a versão em Rust do `balanca-agent` em Python que acompanha o sistema web: as
**mesmas rotas, os mesmos campos JSON e os mesmos códigos de erro**, então o
sistema funciona igual com qualquer um dos dois. A diferença é que este é um
único `.exe`, sem instalar Python nem dependências, e tem **janela própria com
tela de Configurações** — nada de editar `.env`.

---

## 1. Usar

Baixe `balanca-agent.exe` (seção *Releases* do GitHub) ou compile (seção 9) e
abra. Na primeira vez ele cria a configuração padrão: balança **simulada** e
API em `http://127.0.0.1:5000`.

A janela tem quatro abas:

| Aba | Para que serve |
|---|---|
| **Balança** | Visor ao vivo (verde = estável, amarelo = estabilizando), Gerar peso (simulada), Aplicar tara, Zerar, alternar simulada ⇄ real |
| **Configurações** | Todos os parâmetros, agrupados como na tela de Configurações do sistema web. **Salvar e aplicar** valida, grava e aplica na hora, sem reiniciar |
| **Integração** | Exatamente o que preencher no sistema web, com botão de copiar, e a resposta atual de `GET /peso` |
| **Log** | Últimas mensagens; o arquivo completo fica em `logs/balanca.log` |

Deixe o agente aberto enquanto o sistema estiver em uso. Marque
**Configurações → Log e Windows → Iniciar com o Windows** para ele abrir
sozinho, minimizado, ao ligar o computador.

## 2. Ligar ao sistema web

No sistema, entre como administrador → menu do usuário → **Configurações do
sistema** → **Balança**:

| No sistema | Valor |
|---|---|
| Usar balança | ligado |
| Endereço do programa de integração | `http://127.0.0.1:5000` |
| Rota de leitura do peso | `/peso` |
| Rota de registro da captura | `/captura` |
| Campos da resposta | `peso`, `estavel`, `unidade`, `sucesso`, `erro` |

Rotas e campos já são os padrões do sistema; normalmente basta o endereço. Use
**Testar conexão** na tela do sistema para conferir. A aba **Integração** do
agente mostra esses valores com a porta que estiver configurada.

**Sistema publicado na internet** (ex.: Vercel): quem lê a balança é o
navegador do balcão, então o endereço do sistema precisa estar em
**Configurações → API local → Origens permitidas** (ex.:
`https://meusistema.vercel.app`). `localhost` e `127.0.0.1` já vêm liberados. O
agente também responde à checagem de *Private Network Access* do Chrome, que
bloqueia sites da internet de chamar `127.0.0.1` sem permissão explícita.

## 3. Configurações

Ficam em `config.toml`, nesta ordem de procura:

1. ao lado do `.exe`, se existir um `config.toml` lá (**modo portátil**: leve a
   pasta num pendrive);
2. senão, `%APPDATA%\BalancaAgent\config.toml`.

Os logs ficam na pasta `logs` do mesmo lugar. O botão **Abrir pasta** da aba
Configurações abre esse diretório.

São os mesmos parâmetros do `.env` do agente Python, com os nomes em
minúsculas:

| Chave (`config.toml`) | No `.env` do Python | Padrão | O que é |
|---|---|---|---|
| `modo_balanca` | `MODO_BALANCA` | `simulada` | `simulada` ou `real` ao abrir |
| `unidade` | `UNIDADE` | `kg` | |
| `host` / `porta_api` | `HOST` / `PORTA_API` | `127.0.0.1` / `5000` | onde a API sobe |
| `origens_permitidas` | `ORIGENS_PERMITIDAS` | `http://localhost,http://127.0.0.1` | CORS; vírgula ou uma por linha; `*` libera tudo |
| `permitir_rede` | `PERMITIR_REDE` | `false` | `false` = só aceita chamadas deste computador |
| `api_token` | `API_TOKEN` | vazio | protege `/zero`, `/tara`, `/modo` e `/simulada/gerar` (header `X-API-Token`) |
| `intervalo_atualizacao` | `INTERVALO_ATUALIZACAO` | `0.5` | segundos entre leituras |
| `tolerancia_estabilidade` | `TOLERANCIA_ESTABILIDADE` | `0.05` | variação máxima para contar como parado |
| `tempo_estabilidade` | `TEMPO_ESTABILIDADE` | `1.5` | segundos parado para ser estável |
| `casas_decimais` | `CASAS_DECIMAIS` | `2` | |
| `sim_peso_minimo` / `sim_peso_maximo` | `SIM_PESO_MINIMO` / `SIM_PESO_MAXIMO` | `0` / `200` | faixa do Gerar peso |
| `sim_peso_inicial` | `SIM_PESO_INICIAL` | `0` | |
| `sim_resolucao` | `SIM_RESOLUCAO` | `0.10` | divisão da balança simulada |
| `porta_com` | `PORTA_COM` | `COM3` | |
| `baudrate` / `bytesize` | `BAUDRATE` / `BYTESIZE` | `9600` / `8` | |
| `paridade` | `PARIDADE` | `N` | `N`, `E` ou `O` |
| `stopbits` | `STOPBITS` | `1` | `1` ou `2` |
| `timeout` | `TIMEOUT` | `1.0` | segundos esperando um frame completo |
| `comando_peso` | `COMANDO_PESO` | vazio | o que enviar para pedir o peso; vazio = balança transmite sozinha |
| `terminador` | `TERMINADOR` | `\r\n` | fim de cada frame |
| `protocolo_regex` | `PROTOCOLO_REGEX` | `([-+]?\d+[.,]?\d*)` | o grupo 1 (ou `(?P<peso>...)`) é o peso |
| `protocolo_regex_estavel` | `PROTOCOLO_REGEX_ESTAVEL` | vazio | ex.: `^ST`; vazio = o agente calcula |
| `fator_divisao` | `FATOR_DIVISAO` | `1` | `012540` com fator `100` = 125,40 |
| `comando_zero` / `comando_tara` | `COMANDO_ZERO` / `COMANDO_TARA` | vazio | vazio = `COMANDO_NAO_SUPORTADO` |
| `log_nivel` | `LOG_NIVEL` | `INFO` | `DEBUG`, `INFO`, `WARNING`, `ERROR` |
| `iniciar_com_windows` | — | `false` | grava/remove o agente em `HKCU\...\Run` |

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
usa o **último** frame completo (o Python usava o mais antigo, e o visor ficava
atrasado), e descarta o pedaço de frame que estava no meio do caminho quando a
porta foi aberta.

**Regex:** é a sintaxe do crate `regex` do Rust, igual à do Python para tudo o
que um frame de balança precisa (grupos, `(?P<nome>...)`, classes, âncoras),
mas sem *lookaround* (`(?=...)`, `(?<=...)`) nem referências para trás (`\1`).

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
| `/captura` | POST | Registra no log que o sistema capturou o peso — `{"peso": 12.5, "origem": "tela de compra"}` |

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
| "API parada" no topo da janela | Porta em uso (outro agente, inclusive o Python, aberto) — troque a porta ou feche o outro |
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
cargo test                # 43 testes, todos sem hardware
cargo build --release     # target/release/balanca-agent.exe (~8 MB, sem console)
```

Parâmetros de linha de comando:

| Parâmetro | Efeito |
|---|---|
| `--minimizado` | abre minimizado (usado pelo *Iniciar com o Windows*) |
| `--sem-janela` | só a API, sem janela |
| `--aba=configuracoes` | abre direto numa aba (`configuracoes`, `integracao`, `log`) |

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
```
