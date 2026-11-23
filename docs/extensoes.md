# Extensões do Chrome — desenho (issue #6)

Desenho aprovado em 23/11/2026; primeira versão implementada (ver §6).

## 1. O que o WebView2 oferece de fato

A documentação oficial da Microsoft descreve só a **gestão** de extensões.
Ela não lista quais APIs `chrome.*` funcionam. A página conceitual que
deveria cobrir isso (`concepts/browser-extensions`) não existe mais (404).

**Documentado** (referência Win32, SDK 1.0.4314, desde 1.0.2210.55):

| Peça | O que faz |
|---|---|
| `ICoreWebView2EnvironmentOptions6::put_AreBrowserExtensionsEnabled` | Liga extensões no **ambiente inteiro**. Padrão FALSE. É fixado na criação do ambiente: conectar a um ambiente já aberto com outro valor falha com `ERROR_INVALID_STATE`. |
| `ICoreWebView2Profile7::AddBrowserExtension(pasta)` | Instala uma extensão **descompactada** (pasta com `manifest.json`). Não há `.crx` nem Chrome Web Store, e a Microsoft diz que isso é de propósito. Com o ambiente desligado, retorna `ERROR_NOT_SUPPORTED`. |
| `ICoreWebView2Profile7::GetBrowserExtensions` | Retorna uma lista instantânea das extensões do perfil. |
| `ICoreWebView2BrowserExtension` | `Id` (igual ao `chrome.runtime.id`), `Name`, `IsEnabled`, `Enable`, `Remove`. |

Pontos que pesam no desenho:

- **Uma instalação por perfil.** O estado fica guardado em cada perfil.
  Cada espaço com perfil próprio precisa receber a extensão separadamente.
- **A pasta não é copiada.** Se o conteúdo dela mudar, a extensão é
  removida do perfil. Por isso o app guarda a sua própria cópia.
- **Não há interface.** A Microsoft diz textualmente que extensões com
  ícone, selo ou popup carregam e funcionam, mas ficam sem ponto de entrada.

**Não documentado** (decide-se num spike, não no achismo):

- Lista de APIs suportadas. A Microsoft não diz nada sobre MV2 ou MV3.
- Se dá para navegar para `chrome-extension://<id>/...` num WebView2.
- InPrivate e desempenho (processos, memória).

Expectativa por categoria, a confirmar no spike:

| Categoria | Expectativa |
|---|---|
| content scripts, `storage`, `runtime`, `i18n`, service worker | provável: rodam no motor, sem depender de interface |
| `declarativeNetRequest`, `webRequest`, `scripting`, `alarms` | provável, mas precisa medir |
| `tabs` / `windows` | parcial: cada WebView2 é uma "aba" isolada e o app não fica sabendo de `tabs.create`, `tabs.update` nem da "aba ativa" |
| `action` / `browserAction` (ícone, selo, `onClicked`), `contextMenus`, `commands`, `omnibox`, `sidePanel`, `notifications` | sem ponto de entrada; dependem do que o app mostrar (ver §2) |

**Spike antes de qualquer interface.** É uma extensão de teste em
`rust/tests/fixtures/ext-probe/` (MV3), dirigida pelo bench. Ela registra
em `chrome.storage` quais APIs respondem e o app lê o resultado com `eval`.
O spike responde, nesta ordem:

1. Content script e service worker rodam?
2. `chrome-extension://<id>/popup.html` abre numa WebView2 nossa?
3. O que `chrome.tabs.query({active:true, currentWindow:true})` retorna
   dentro desse popup?

Se o item 2 falhar, cai o popup inteiro (§2) e a primeira versão fica só
com extensões sem interface: bloqueadores, temas de página, scripts.

## 2. Como mostrar o popup

O app lê o `manifest.json` (`action.default_popup` no MV3,
`browser_action.default_popup` no MV2) e abre essa página numa janela dele.

- **A janela.** Popup sem moldura, `WS_EX_TOOLWINDOW`, no mesmo molde do
  painel do Peek (`peek.rs`). Tem um controller WebView2 próprio, criado
  no **mesmo perfil** da aba ativa e apontado para
  `chrome-extension://<id>/<default_popup>`.
- **Tamanho.** Segue o conteúdo, como no Chrome: depois de carregar, um
  `eval` mede `document.documentElement.scroll{Width,Height}`. O limite é
  800×600 e o mínimo 25×25.
- **Posição.** Ancorado embaixo do botão (§3). Fica dentro do monitor.
- **Fechamento.** Fecha com Esc, quando a janela perde o foco
  (`WM_ACTIVATE` inativo) ou com `window.close()` da própria página
  (`WindowCloseRequested`). Ao fechar, o controller é **descartado**:
  popup fechado não custa nada.
- **Animação.** Entra e sai com um fade curto, nas regras do CLAUDE.md
  (cadência da tela e respeito ao "animações desligadas"). Usa o que a
  issue #12 criar; aqui não nasce outro mecanismo.
- **"Aba ativa".** Popups que chamam `tabs.query({active:true})` para
  saber o site atual são a maioria. Se o spike mostrar que o WebView2 não
  responde isso direito, esses popups ficam marcados como limitação
  conhecida. Não vamos tentar emular a API `tabs` nesta versão.
- **O que fica de fora.** Selo (`setBadgeText`), troca dinâmica de ícone
  (`setIcon`), `action.onClicked` em extensões sem popup (o app não tem
  como disparar esse evento), menus de contexto de extensão, atalhos
  `commands` e `sidePanel`.

## 3. Onde fica o botão

Na barra, entre o campo de endereço e os botões da janela. É um único
botão quadrado do mesmo tamanho de voltar/avançar/recarregar (`NAV`), com
o glifo de peça de quebra-cabeça da fonte de ícones do Windows, a mesma
dos outros botões. Fica na barra e não na coluna porque o popup age sobre
a página atual, e a página fica sob a barra.

- **Aparece só quando há o que abrir:** pelo menos uma extensão ligada
  **com** popup. Sem isso, o botão nem é desenhado e a barra fica como
  hoje.
- **Uma extensão com popup:** o clique abre o popup direto.
- **Várias:** o clique abre um menu Win32 com nome e ícone de cada uma.
  O menu segue o tema, como o menu da aba (`TrackPopupMenu`). Escolher
  uma abre o popup dela.
- **Gerenciar** (instalar, ligar, desligar, remover) fica numa seção da
  `search://settings`, não numa página nova. A seção só aparece com o
  recurso ligado. "Adicionar" pede uma pasta descompactada pelo diálogo
  do Windows.
- O que só o mouse alcança (botão, menu, popup) entra na lista de teste
  manual.

## 4. Custo zero sem extensão

Regra: **desligado não cria nada**, e **ligado sem extensão também não**.

| Estado | O que existe |
|---|---|
| Preferência `extensions` desligada (padrão) | Nada. `AreBrowserExtensionsEnabled` não é chamado, o botão não existe e nenhum arquivo novo é criado. |
| Ligada, sem extensão | Igual ao desligado. O ambiente continua com o flag FALSE e só aparece a seção em Settings. |
| Ligada, com ≥1 extensão | O flag fica TRUE ao criar o ambiente. Surge `Extensions\` na pasta de dados, com as cópias e o `extensions.json` (id, nome, popup, ligada). O botão aparece só se houver popup. |

Como o flag é fixo por ambiente:

- **A primeira instalação exige reiniciar o app.** Settings avisa
  "reinicie para ativar". Recriar o ambiente com abas abertas derrubaria
  todas as páginas, e o ganho não compensa.
- **Ao remover a última extensão**, o flag volta a FALSE no próximo
  início. Fica só a pasta vazia, que o app também apaga.

Instalação por perfil, sob demanda: quando um controller nasce num perfil
(espaço) pela primeira vez na sessão, o app chama `GetBrowserExtensions`
e adiciona as que faltam. Isso acontece uma vez por perfil por sessão,
fora do caminho da primeira página (depois de `NavigationCompleted`).

**InPrivate não recebe extensões**, como no padrão do Chrome. A aba
privada continua sem nada de terceiros.

**Medição obrigatória** com o bench, contra a referência da Fase 2:

| Medida | Condição |
|---|---|
| exe | delta ≤ ~10 KB |
| memória e primeira página | **iguais** à referência com o recurso desligado e com ele ligado sem extensão |
| memória e primeira página | **reportar** com 1 extensão (não há meta: é o custo da extensão) |

## 5. Primeira versão (escopo proposto)

1. Spike (§1), com o resultado anotado aqui.
2. Preferência `extensions` (desligada) e seção em Settings: adicionar
   pasta, ligar, desligar e remover. O app copia a pasta para
   `Extensions\<id>\`.
3. Flag no ambiente só com ≥1 extensão, e instalação sob demanda por perfil.
4. Botão e popup, **somente se** o item 2 do spike passar.
5. Testes: crate pequena `extensions` (ler o manifest, achar o popup,
   lista em JSON) e um teste no bench com a `ext-probe`.

Fica de fora:

- `.crx` e Chrome Web Store.
- Selo, `onClicked`, menus de contexto de extensão, atalhos e `sidePanel`.
- Extensões no InPrivate.
- Sincronizar extensões entre perfis além da instalação sob demanda.
- Atualização automática das extensões.

Ponto de atenção: bloqueadores (uBlock e similares) se sobrepõem ao
escudo nativo (`shield`). Nada impede os dois ao mesmo tempo, mas a
seção de Settings pode sugerir desligar um deles.

## 6. Resultado do spike (23/11/2026, WebView2 do Edge 157)

Extensão de teste `rust/tests/fixtures/ext-probe/` (MV3), dirigida pelo bench.

| Pergunta | Resposta |
|---|---|
| Content script roda? | **Sim**, em `document_start`, com `<all_urls>`. |
| Service worker roda e troca mensagens? | **Sim**: `runtime.sendMessage` vai e volta; `sender.tab` traz id e URL da aba. |
| `chrome-extension://<id>/popup.html` abre numa WebView2 nossa? | **Sim**. O popup roda com `storage`, `tabs`, `alarms` e `runtime.id`. |
| `tabs.query({active, currentWindow})` no popup | Devolve **o próprio popup**, não a página da aba. É a limitação prevista: popups que agem sobre "a aba atual" não acham a página. |
| APIs presentes no service worker | `tabs`, `windows`, `storage`, `alarms`, `scripting`, `declarativeNetRequest`, `contextMenus`, `action` |
| APIs ausentes | `commands`, `notifications`, `sidePanel`. `webRequest` não foi pedido no manifest. |

O botão e o popup entraram (item 4 do §5 passou).

Ficou diferente do desenho:

- O menu de várias extensões mostra **só o nome**, sem ícone.
- O popup entra **sem animação**. O fade espera o mecanismo da issue #12.
- Links que o popup abre (`window.open`, `target=_blank`) viram aba depois
  da aba atual.

## 7. Medição (23/11/2026)

Mediana de 5 inícios quentes, com a mesma sessão (uma aba no
example.com). "App" é a memória privada do SearchWin.exe; "WebView2" é a
soma dos processos do motor.

| | exe | primeira página | app | WebView2 |
|---|---|---|---|---|
| base `b3e5386` | 1 336 320 B | 365 ms | 3,4 MB | 308 MB |
| recurso desligado | 1 377 280 B¹ | 369 ms | 3,4 MB | 308 MB |
| ligado, 0 extensão | 1 377 280 B¹ | 366 ms | 3,4 MB | 308 MB |
| ligado, 1 extensão (`ext-probe`) | 1 377 280 B¹ | 405 ms | 3,4 MB | 357 MB |

¹ Antes de tirar o `serde` derive. Depois: 1 372 672 B. Os tempos e a
memória foram medidos antes desse corte, que não muda o caminho de início.

- Desligado e ligado sem extensão ficaram **iguais à base**, dentro do ruído.
- **O exe cresceu 40 KB**, acima da meta de ~10 KB do §4. Investigado com
  o mapa do linker (`/MAP`):
  - O `serde` derive da lista custava ~9 KB e foi trocado por
    `serde_json::Value`, que o app já carrega. O exe caiu para
    **1 372 672 B (+36 KB)**.
  - O que sobra é o próprio recurso: ~31 KB de código e ~5 KB de dados.

    | Parte | Código |
    |---|---|
    | `extensions.rs`, a crate e os 5 handlers COM de extensão | 18,3 KB |
    | ops `extensions.*` embutidas na ponte | 5,6 KB |
    | ligações no `main.rs` | 2,1 KB |
    | `page.rs` | 1,6 KB |
    | bench | 1,4 KB |
    | remanejamento do LTO | 1,3 KB |

  - Os dados são os textos da Settings e as vtables.
  - Não há gordura grande para cortar sem tirar uma parte do recurso. **A
    meta de ~10 KB era otimista** para sincronização por perfil, mais o
    popup, mais a gestão na Settings.
- Com 1 extensão: +40 ms na primeira página e +49 MB no WebView2 (o
  service worker e o processo da extensão). É o custo da extensão.

Testes: 6 novos na crate `extensions`, mais as ops `extensions.*` nos
testes da ponte (`internal.rs`). Pelo bench rodaram o spike, o fluxo da
Settings (ligar, desligar, remover, varredura no início seguinte) e o
`window.close()` do popup.

**Teste à mão** (só o mouse alcança):

- o botão de peça aparece e some com a chave e com aba privada;
- clicar abre o popup sob o botão;
- com 2 ou mais extensões, o menu aparece;
- clicar fora do popup fecha;
- clicar no botão com o popup aberto fecha, e não reabre;
- Esc fecha;
- o diálogo "Adicionar pasta…" abre.

## Fontes

- [ICoreWebView2EnvironmentOptions6](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2environmentoptions6)
- [ICoreWebView2Profile7](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2profile7)
- [ICoreWebView2BrowserExtension](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2browserextension)
- [WebView2Feedback #98 — Add Ons or Extensions](https://github.com/MicrosoftEdge/WebView2Feedback/issues/98)
- [WebView2Feedback #4650 — AreBrowserExtensionsEnabled](https://github.com/MicrosoftEdge/WebView2Feedback/issues/4650)
