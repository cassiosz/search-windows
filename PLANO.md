# Plano — versão Windows do Search

Origem: https://github.com/driceroland/Search (v1.0.14, clonado em `upstream/`,
MIT; o nome "Search" e o ícone são da Office Commun — renomear antes de
distribuir). O app de Mac é Swift + WebKit + AppKit; o próprio roadmap diz que
não há nada a levar para o Windows. Logo, **é uma reescrita**.

**Prioridade que manda em tudo: rápido, leve, interface mínima** — uma coluna
com o campo e as abas, a página como um cartão. Nada entra se piorar o tempo
até a primeira página ou a memória sem motivo explícito.

**Animações fluidas e bonitas** fazem parte da identidade e diferenciam o app
dos outros navegadores. As regras:

- animação curta, com curva suave, sem engasgo;
- acompanhar a taxa de atualização da tela, nunca um timer fixo;
- respeitar "animações desligadas" do Windows;
- nunca atrasar a primeira página nem pesar na memória.

## Decisões

| Tema | Escolha | Por quê |
|---|---|---|
| Linguagem | **Rust** | ganhou a comparação (abaixo) |
| Interface | **Win32 puro + GDI**, campo = controle EDIT nativo | sem runtime; acentos/IME do Windows de graça |
| Motor | **WebView2** (Edge do sistema), via `webview2-com` 0.39 + `windows` 0.62 | o equivalente ao "WebKit que já vem no Mac" |
| Páginas internas | `search://…` servidas pelo próprio app (HTML no executável) | como o upstream, que tornou Ajustes/Histórico páginas |
| Nome | provisório, uso pessoal | definir antes de distribuir |
| Idioma | pt-BR + en (segue o Windows) | |
| Commit | só quando o dono do projeto pedir | |

### Comparação que decidiu (Fase 0, mesma janela, 17/11/2026)

| | C# + WinUI 3 (`src/`, guardado) | Rust + Win32 (`rust/`) |
|---|---|---|
| Tamanho | 103 MB, 250 arquivos | **442 KB, 1 arquivo** |
| Memória do app | 80 MB | **3,4 MB** |
| WebView2 (6 processos) | ~180 MB | ~171 MB |
| Primeira página | ~640 ms | **~360 ms** |

## Estrutura (`rust/`, workspace cargo)

| Onde | O quê | Porta de (upstream) |
|---|---|---|
| `src/main.rs` | janela, coluna (GDI), campo, atalhos, sessão/histórico no estado | Browser, TabBar, Side, Omnibox |
| `src/page.rs` | um WebView2 por aba, criado ao mostrar, descartado ao dormir/fechar | Tab |
| `src/shield.rs` + `crates/shield` | bloqueador: filtros WebView2 só p/ domínios bloqueados + CSS no início do documento | Shield, YouTubeAds, Registrable |
| `crates/tabs` | modelo de abas, pins, sono, `session.json` | Session, Sleep, Pins, Closing |
| `crates/history` | histórico, sugestões tolerantes a erro, completar endereço | History, Fuzzy |
| `src/drive.rs` + `crates/bench` | bench: named pipe só do usuário, com `SEARCHWIN_BENCH=1` | Bench, `./bench` |
| `src/address.rs`, `engine.rs`, `omnibox.rs` | endereço × busca, motores | Address, Engine |

Dados em `%LOCALAPPDATA%\SearchWin-rs\` (session.json, history.json,
shield.json, Pages\ do WebView2). Nada sai do PC além das páginas pedidas.

**Testes**: `cargo test --workspace` (127 hoje). **Teste do app**: só pelo
bench (`SEARCHWIN_BENCH=1`, cliente `bench`) — nunca teclas/mouse simulados
no Windows (já mandou teclas para outra janela do usuário).

## Fases

### Fase 0 — Base ✅
Janela, campo, WebView2, medição. Feita em C# e em Rust para comparar.

### Fase 1 — Núcleo (v1) ✅ (18/11/2026)

Release x64, `opt-level = "z"`: **exe 854 KB, app ~3,5 MB, 1ª página ~338 ms**.
193 testes, clippy limpo. Tudo provado pelo bench, em pastas de teste
separadas (`SEARCHWIN_DATA`), sem tocar nos dados reais.

| Item | Atalho |
|---|---|
| Abas, pins, sessão restaurada (abas voltam dormindo), sono após 30 min | Ctrl+T/W, Ctrl+Shift+T, Ctrl+1–9, Ctrl+Tab |
| Campo: endereço × busca; sugestões de favoritos + histórico (tolera erro de digitação); completar ao digitar | Ctrl+L |
| Trocar de aba pelo nome | Ctrl+K |
| Aba privada (InPrivate, cookies próprios, nada gravado) | Ctrl+Shift+N |
| Shield (rastreadores bloqueados antes da requisição; CSS no início do documento) | — |
| Esconder elementos, por site, antes do 1º quadro; ver o que está escondido | Ctrl+Shift+H, Ctrl+Shift+U |
| Modo leitura (tema claro/escuro/sistema guardado) | Ctrl+Shift+R |
| Páginas internas `search://settings`, `history`, `bookmarks`, `downloads` (ponte só para páginas `search://`) | Ctrl+, Ctrl+H Ctrl+Shift+O Ctrl+J |
| Favoritos: pastas, renomear, mover, abrir pasta inteira, importar do Chrome/Edge | Ctrl+Shift+B |
| Downloads na pasta Downloads: progresso, pausar/retomar/cancelar, nomes "x (1).pdf" | — |

Ganchos de teste: `SEARCHWIN_DATA`, `SEARCHWIN_BENCH`, `SEARCHWIN_TIMING`, `SEARCHWIN_THEME`,
`SEARCHWIN_DOWNLOADS`, `SEARCHWIN_IMPORT_FILE`.

Pendências (pequenas):
- Mouse na coluna (clique, ×, clique do meio, rolagem) e "abrir arquivo" /
  "mostrar na pasta" dos downloads: sem teste automático — testar à mão.
- Aba tocando áudio ainda não impede o sono.
- ~~1º Ctrl+Shift+B só adiciona~~: agora abre o cartão do favorito (nome,
  pasta, nova pasta, Remover, OK; Esc/Enter/clique fora fecham), como no
  upstream (#3). +14 KB de exe; nada é criado até o primeiro Ctrl+Shift+B.
  Menu de pasta, hover, sombra e tema do cartão: testar à mão.
- Completar endereço em cinza ainda não completa para favoritos.
- Sessão do Mac: `pinID`/`groupID` por aba não são lidos.

### Fase 2 — Organização ✅ (18/11/2026)

Release x64: **exe ~1050 KB, app ~3,6 MB, 1ª página ~307 ms**. 280 testes,
clippy limpo. Tudo **desligado por padrão**, como no upstream — desligado,
não cria nada (nem janela, nem script).

| Item | Como |
|---|---|
| Spaces: abas separadas, nome/cor/ícone, chips no pé da coluna, cor tingindo a coluna; sessão por Space (`session-<id>.json`) | Ajustes › Spaces; Ctrl+Alt+1–9, botões laterais do mouse |
| Levar links de sites escolhidos para um Space | Ajustes › Spaces |
| Perfis: cookies, histórico e favoritos próprios (perfil WebView2 por perfil; `Profiles\<id>\`) | Ajustes › Perfis |
| Split View: até 4 páginas, divisória arrastável, foco com contorno, volta após reiniciar | Ajustes › Abas; Ctrl+Alt+N, Ctrl+Alt+←/→, Alt+clique, menu da aba |
| Vídeo flutuante: a página vai para uma janela sempre no topo, sem recarregar | Ctrl+Shift+P; "vídeo segue para outro app" em Ajustes |
| Peek: link abre num painel sobre a página; vira aba sem recarregar | Ajustes › Navegação; Shift ou Alt + clique; Esc, Ctrl+Enter |

Crates novas: `crates/spaces`, `crates/split`.

Sem teste automático (mouse) — testar à mão: arrastar/duplo clique na
divisória, Alt+clique, menu da aba, chips dos Spaces, controles da janela
flutuante (arrastar, redimensionar, voltar, ×), botões do peek e o
Shift/Alt+clique de verdade.

Deixado de fora do upstream: janela pequena para links de outros apps
(Little), acoplar/arremessar o vídeo flutuante e seus botões de
tocar/pular, esqueleto de carregamento do peek e "peek ao lado".

### Fase 3 — Senhas e importação ✅ (21/11/2026)

Release x64: **exe ~1300 KB** (+250 KB: Gerenciador de Credenciais, WinRT do
Windows Hello, SQLite/sessão da importação, diálogos de arquivo), app ~3,6 MB,
1ª página ~310–333 ms (ruído; sem A/B na mesma sessão). 321 testes, clippy
limpo, `cargo audit` sem alertas. Revisão de segurança feita por agente
separado: 3 médios + 5 leves, corrigidos; a segunda revisão achou mais 5
leves, também corrigidos.

| Item | Como |
|---|---|
| Cofre no Gerenciador de Credenciais (`SearchWin:host:usuário`) | — |
| Contas oferecidas sob o campo de login; preencher com um clique | só no host exato (com porta); quadro só se for da mesma origem |
| Barra para salvar/atualizar/nunca após o login | nada em aba privada; https nunca vira http |
| `search://passwords`: ver, copiar, editar, apagar, importar/exportar CSV | Ctrl+Alt+L; ver/copiar/exportar pedem Windows Hello |
| Copiar senha: fora do histórico da área de transferência, limpa em 30 s | — |
| Importar de Chrome/Edge/Brave/Arc/Firefox: favoritos, histórico, abas, Spaces e pins do Arc | `search://import` |
| Passkeys | ficam com o Windows (WebAuthn do WebView2 ligado) |

Crates novas: `crates/vault`, `crates/import`. Ganchos de teste novos:
`SEARCHWIN_VAULT_PREFIX`, `SEARCHWIN_IMPORT_BASES`, `SEARCHWIN_VERIFY` e
`SEARCHWIN_BENCH_FILE` (os dois últimos só com `SEARCHWIN_BENCH=1`; bench
sem prefixo de teste recusa o cofre).

Decisões: sem Windows Hello configurado, ver/copiar/exportar são recusados
com aviso (sem caminho alternativo pela senha do Windows). Senhas do Chrome
só por CSV (criptografia "app-bound"). Sem "salvar para o site todo": conta
vale só para o host exato — login em subdomínio irmão não é oferecido.

Testar à mão: lista de contas e barra de salvar (aparência), clique na lista
(ignorado nos primeiros 500 ms; apertar numa linha e soltar em outra não
faz nada; lista fica dentro da página nas bordas; o mesmo vale para os
botões da barra de salvar), prompt real do Windows
Hello, copiar + limpeza em 30 s, diálogos de arquivo do CSV.

Deixado de fora: ajuste "oferecer salvar senhas" (sempre ligado), pastas de
pins do Arc (entram como pins soltos).

### Animações ⬜ (transversal, antes de distribuir)

O GDI puro não anima de forma suave. Para chegar lá, a coluna e as transições
passam para **DirectComposition**, que já vem no Windows e não exige runtime
novo. Medir antes e depois: exe, memória, primeira página.

O que anima:
- abrir, fechar e trocar aba;
- trocar de Space (cor da coluna);
- sugestões do campo;
- Split View e divisória;
- peek e vídeo flutuante;
- barra de salvar senha e lista de contas.

### Fase 4 — Distribuição ⬜
Extensões do Chrome (WebView2 tem suporte parcial e sem popup — desenhar),
sync por pasta (OneDrive/Dropbox, mesmo modelo do upstream), atualizador e
instalador, nome e ícone definitivos.

### Fase 5 — Extras ⬜
Tradução (sem API local equivalente no Windows), IA local (`search-ai-engine`
do upstream usa llama.cpp e compila no Windows).

## Riscos

- **Extensões**: WebView2 cobre só parte da API do Chrome.
- **Captcha do Google**: visto uma vez num perfil novo com buscas por script;
  não se repetiu em 5 buscas. O WebView2 se anuncia como Edge (`Edg/157`) e
  acrescenta a marca "Microsoft Edge WebView2" nos client hints, que não dá
  para tirar por API.
- **Mouse e desenho à mão**: arrastar abas, acessibilidade e animações custam
  mais que num framework pronto — é o preço dos 3,7 MB.
