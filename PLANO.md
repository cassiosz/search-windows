# Plano — versão Windows do Search

Origem: https://github.com/driceroland/Search (v1.0.14, clonado em `upstream/`,
MIT; o nome "Search" e o ícone são da Office Commun — renomear antes de
distribuir). O app de Mac é Swift + WebKit + AppKit; o próprio roadmap diz que
não há nada a levar para o Windows. Logo, **é uma reescrita**.

**Prioridade que manda em tudo: rápido, leve, interface mínima** — uma coluna
com o campo e as abas, a página como um cartão. Nada entra se piorar o tempo
até a primeira página ou a memória sem motivo explícito.

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
- 1º Ctrl+Shift+B só adiciona (upstream abre um cartão para renomear).
- Completar endereço em cinza ainda não completa para favoritos.
- Sessão do Mac: `pinID`/`groupID` por aba não são lidos.

### Fase 2 — Organização ⬜
Spaces, perfis, Split View, vídeo flutuante (PiP), peek.

### Fase 3 — Senhas e importação ⬜
Cofre no Gerenciador de Credenciais do Windows, passkeys (WebAuthn/Windows
Hello pelo WebView2), importar de Chrome/Edge/Arc/Brave (senhas do Chrome só
por CSV — criptografia "app-bound").

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
