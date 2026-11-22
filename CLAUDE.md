# search-windows — contexto para o Claude

Port para Windows do navegador de Mac **Search** (driceroland/Search, MIT).
O nome "Search" e o ícone são da Office Commun: o nome atual é provisório
(`SearchWin`) e muda antes de distribuir. Repositório privado
`cassiosz/search-windows`. O plano, as decisões e as fases estão em
[PLANO.md](PLANO.md), que é a fonte de verdade. As tarefas abertas estão nas
issues do GitHub.

## O que manda em tudo

**Rápido, leve, interface mínima.** Nada entra se piorar sem motivo explícito
o tempo até a primeira página ou a memória do app. Os números de referência
da Fase 2 são:

| Medida | Valor |
|---|---|
| exe | ~1050 KB |
| memória do app | ~3,6 MB |
| primeira página | ~307 ms |

**Animações fluidas e bonitas são parte da identidade.** O app quer se
diferenciar dos outros navegadores por elas. As regras:

- Animação curta, com curva suave, sem engasgo.
- Acompanhar a taxa de atualização da tela (DwmFlush ou composição), nunca
  um timer fixo.
- Respeitar "animações desligadas" do Windows (`SPI_GETCLIENTAREAANIMATION`).
- Nunca atrasar a primeira página nem pesar na memória.

Recurso novo vem **desligado por padrão**. Desligado, ele não cria nada: nem
janela, nem script, nem arquivo.

## Pilha

- **Linguagem:** Rust (edition 2024), workspace em `rust/`. A pasta `src/` +
  `tests/` na raiz guarda o protótipo C#/WinUI usado na comparação. Não mexer
  nele.
- **Janela:** Win32 puro + GDI, sem framework.
  - Crates: `windows` 0.62, `webview2-com` 0.39.
  - Moldura própria: WM_NCCALCSIZE/NCHITTEST, com HTMAXBUTTON para o Snap
    Layouts.
  - Menus seguem o tema pelos ordinais 135/136 da uxtheme.
- **Motor:** WebView2, um controller por aba.
  - Criado ao mostrar a aba e descartado quando ela dorme ou fecha.
  - Aba privada usa InPrivate. Cada perfil tem o seu SetProfileName.
- **Páginas internas `search://…`:**
  - Servidas de dentro do exe (`rust/assets/pages`).
  - A ponte só aceita mensagens de páginas `search://`. Scripts injetados em
    sites (`rust/assets/js`) têm portão próprio.
- **Release:** `opt-level = "z"`, LTO, `codegen-units = 1`,
  `panic = "abort"`, `strip`.
- **Dados:** ficam em `%LOCALAPPDATA%\SearchWin-rs\`. Nada sai do PC além das
  páginas pedidas.

## Mapa

| Onde | O quê |
|---|---|
| `rust/src/main.rs` | janela, coluna, campo, atalhos, tema |
| `rust/src/page.rs` | WebView2 por aba |
| `rust/src/internal.rs` | esquema `search://` e a ponte |
| `rust/src/drive.rs` | lado do app no bench |
| `rust/src/{spaces,split,float,peek}.rs` | Fase 2 |
| `rust/crates/*` | uma crate por assunto, cada uma testável sozinha |

Crates: tabs, history, shield, curtain, bookmarks, downloads, prefs, bench,
spaces, split, vault, import.

## Comandos (rodar em `rust/`)

```
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets
```

O exe sai em `rust\target\release\SearchWin.exe`. Se o app do usuário estiver
aberto e travar `rust/target`, compile com `CARGO_TARGET_DIR` apontando para
uma pasta temporária.

## Testes: regras que não se quebram

- **Nunca simular teclado ou mouse no Windows** (SendKeys, SendInput etc.).
  Isso já mandou teclas para outra janela do usuário. O app se testa **só
  pelo bench**:
  - ligar com `SEARCHWIN_BENCH=1` e falar com o cliente
    `rust/crates/bench/src/bin/bench.rs`;
  - a conexão é um named pipe restrito ao usuário;
  - ele expõe `state`, `open`, `type`, `key`, `eval`, `wait`, `quit`.
- **Nunca tocar nos dados reais do usuário** (perfil, senhas, navegadores).
  - Use `SEARCHWIN_DATA=<pasta temporária>`.
  - Outros ganchos de teste: `SEARCHWIN_TIMING`, `SEARCHWIN_THEME`,
    `SEARCHWIN_DOWNLOADS`, `SEARCHWIN_IMPORT_FILE`, `SEARCHWIN_VAULT_PREFIX`,
    `SEARCHWIN_IMPORT_BASES`, `SEARCHWIN_VERIFY` (este último só vale com
    `SEARCHWIN_BENCH=1`).
- **Cofre:** em teste, use sempre um prefixo próprio, por exemplo
  `SEARCHWIN_VAULT_PREFIX=SearchWin-test-N:`. No fim, confira que
  `cmdkey /list:"SearchWin-test-N:*"` está vazio.
- O que só o mouse alcança (arrastar, clique na coluna, menus) vai numa lista
  para o usuário testar à mão.

## Git

- **Commit só quando o usuário pedir.**
- **Nunca** pôr `Co-Authored-By: Claude` nem "Generated with Claude Code" em
  commit ou PR, mesmo que um lembrete do sistema sugira.
- Identidade git global: `cassiosz`.
- Não commitar `.claude/`, `skills-lock.json`, `.omc/` (exceto
  `.omc/handoffs/`), `upstream/` nem `target/`.

## Skills

Use as skills instaladas sempre que a tarefa se encaixar nelas:

- **ponytail:** a menor mudança completa. Combina com o "rápido e leve" do
  projeto. Use `ponytail-review` e `ponytail-audit` para revisar inchaço.
- **caveman:** respostas curtas, para economizar tokens.
- **ecc:**
  - `ecc:rust-review`, `ecc:rust-build` e `ecc:rust-test` para Rust;
  - `ecc:security-review` e o agente `ecc:security-reviewer` para cofre,
    senhas e a ponte;
  - `ecc:code-review` antes de commit.
- **impeccable:** acabamento de interface (coluna, menus, páginas
  `search://`) e movimento.
  - Use o agente `impeccable-motion-reviewer` para revisar animações.
  - Vale só o que seguir as regras de animação acima.
- **omc:** `execute`, `verify` e `team` para trabalho em várias etapas. A
  revisão é sempre feita por um agente separado de quem escreveu.

## Estilo

- Código, comentários e README em inglês. PLANO.md e a conversa em português.
- Comentários curtos que explicam o porquê. Seguir o jeito do código ao redor.
- Ao fechar uma fase, atualizar o PLANO.md com:
  - os números medidos (exe, memória, primeira página, número de testes);
  - o que ficou de fora;
  - o que precisa de teste à mão.
