# Design do Search (macOS) — referência para o port

Descrição do design visual e das animações do **Search** instalado em
`/Applications/Search.app` (Office Commun, build `202611202103`), para servir de
referência ao port Windows.

**Fontes:**
- **Observado:** o app instalado, inspecionado em 21/11/2026, no modo escuro, com as
  abas no topo e a opção "Shorter tab bar" ligada.
- **Código:** valores exatos (cores, métricas, springs, durações) tirados do
  código público do upstream (`driceroland/Search`, branch `main` de 21/11/2026).
  Os principais arquivos são `Design.swift`, `Omnibox.swift`, `TabBar.swift`,
  `Side.swift`, `Stage.swift`, `Peek.swift`, `LiquidTab.swift` e `Float.swift`.
  Quando a tela e o código não batem, vale o que está na tela.

---

## 1. Princípios

1. **A página é o chão.** O navegador quase não desenha nada. Tudo é cinza
   neutro, sem cor de destaque. Só há duas cores: verde (conexão segura) e
   âmbar (insegura).
2. **Um campo, as abas e a página.** Não há barra de endereço fixa. O campo de
   endereço aparece no centro quando é chamado (⌘L, ⌘T) e some depois.
3. **Pouco movimento, sempre o mesmo.** Há uma spring para o que *se move* entre
   dois lugares e outra para o que *chega ou sai*. Repetir as mesmas duas em
   todo o app é o que faz ele parecer uma peça só.
4. **Nada se reorganiza enquanto você lê.** As abas têm largura fixa, então o "×"
   cai sempre no mesmo lugar. A lista de sugestões fica pendurada *abaixo* do
   campo e nunca o empurra.
5. **Acessibilidade em primeiro lugar.** Com Reduce Motion (macOS) ou "Reduce
   animations" (Ajustes do Search) ligados, toda transição fica instantânea.

---

## 2. Cores

Cada cor é um **par claro/escuro** em escala de cinza pura (`NSColor(white:)`).
Elas se resolvem sozinhas conforme a aparência da janela.

| Token | Claro (white) | Escuro (white) | Uso |
|---|---|---|---|
| `ground` | 1.00 | 0.11 | Fundo da janela, campo, popovers |
| `ink` | 0.09 | 0.93 | Texto principal |
| `muted` | 0.55 | 0.58 | Texto secundário, título de aba inativa |
| `faint` | 0.83 | 0.32 | Placeholder, grip em repouso |
| `hairline` | 0.91 | 0.20 | Bordas de 1 px |
| `wash` | 0.937 | 0.175 | Aba ativa |
| `hover` | 0.965 | 0.15 | Item sob o ponteiro |
| `frame` | 0.955 | 0.075 | Moldura em volta do "card" da página (layout em coluna) |
| `resting` | 0.80 | 0.30 | Semáforos desenhados à mão com o app em segundo plano |
| `safe` | rgb(0.08, 0.50, 0.24) | rgb(0.29, 0.87, 0.50) | Conexão segura (green‑700 / 400) |
| `unsafe` | rgb(0.71, 0.33, 0.04) | rgb(0.98, 0.75, 0.14) | Conexão insegura (amber‑700 / 400) |

Em hex, no modo escuro: `ground` ≈ `#1C1C1C`, `ink` ≈ `#EDEDED`,
`muted` ≈ `#949494`, `hairline` ≈ `#333333`, `wash` ≈ `#2D2D2D`.

**Cinzas translúcidos (`Greys`).** Linhas de aba, pins e botões não usam um
cinza opaco. Usam preto (no claro) ou branco (no escuro) com alpha. Assim, sobre
a cor ou o gradiente de um Space, eles herdam o tom em vez de virarem uma
mancha cinza. Exemplo no chão branco:

- `wash`: preto 6,3 % / branco 7,3 %
- `hover`: preto 3,5 % / branco 4,5 %
- `pinLive`: preto 10 % / branco 11,2 %

**Aparência.** Light, Dark ou Auto, aplicada no `NSApp`. Assim, todas as
janelas, painéis e páginas concordam. Nos Ajustes, cada opção é uma miniatura
da janela e a selecionada ganha um contorno de 2 px.

---

## 3. Tipografia

- A fonte é a do sistema (SF Pro) em toda parte. Não há fonte própria.
- Tamanhos usados com mais frequência:
  - **13 pt:** campo, linhas de sugestão.
  - **12 pt:** título de aba, aba "levantada".
  - **11 pt:** legendas.
  - **10 pt medium:** rótulos.
  - **8–9 pt semibold:** badges.
- Nos Ajustes:
  - Título de seção: ~17 pt semibold, com ícone.
  - Cabeçalho de grupo: 10 pt semibold, em CAIXA ALTA e espaçado ("YOUR BROWSER").
  - Linha: título em 13 pt `ink` e descrição em 11 pt `muted`.

---

## 4. Métricas e formas

| Métrica | Valor |
|---|---|
| Altura da faixa de abas | **52 pt**, ou **36 pt** com "Shorter tab bar" (abas no topo) |
| Início da primeira aba | x = 100. Os semáforos vão de 19 a 79, então sobra o mesmo ar dos dois lados. |
| Largura da aba | **186 pt fixa**. Abaixo de 80 mostra só o ícone; o mínimo é 36 (depois disso a faixa rola). |
| Gap entre abas | 2 pt |
| Pin (aba fixada) | quadrado de 30 pt, com uma letra ou o favicon |
| Botão "+" | 30 pt |
| Voltar / avançar / recarregar | 3 × 26 pt, no canto direito da faixa, ao lado do botão de favorito |
| Campo de endereço | **560 pt** de largura e raio **14** |
| Coluna lateral (abas no lado) | 232 pt (mín. 176, máx. 440) |
| "Card" da página com a coluna | margem de 8 pt e raio 10, borda `primary` 8 % e sombra preta 7 % (r6, y1) |
| Raio de abas e linhas | **9** (é o mais comum no código) |
| Grip de redimensionar | pílula de 4 × 28 pt, que cresce para 4 × 40 sob o ponteiro |

- **Cantos.** Sempre `RoundedRectangle(style: .continuous)`, ou seja, squircle.
- **Bordas.** Sempre de 1 px na cor `hairline`.
- **Sombras.** São muito suaves, preto entre 6 % e 25 %, com raio grande:

| Elemento | Sombra |
|---|---|
| Campo de endereço | 6 %, r24, y8 |
| Lista de sugestões | 7 %, r20, y6 |
| Aba carregada (arrastada) | 18 %, r12, y4 |
| Peek | 25 %, r30, y10 |

---

## 5. Componentes

### 5.1 Janela e faixa de abas (abas no topo)
- A barra de título cresce até a altura da faixa. Assim, os semáforos descem
  junto e ficam alinhados com as abas.
- A aba ativa é uma pílula `wash` (raio 9) com o título em `ink`. As inativas
  não têm fundo e usam `muted`. No hover aparece uma pílula `hover`.
- O ícone de cada aba é o favicon de 14–15 pt ou uma letra (monograma).
- Os botões ⟨ ⟩ ↻ e o de favorito ficam à direita e aparecem apagados
  (`faint`) quando estão inativos.

### 5.2 Campo central (Omnibox)
Observado em uma aba nova:

- **Visual.** O campo fica centralizado e um pouco acima do centro (60 pt de
  padding embaixo, porque o centro exato "parece baixo"). Tem 14 pt de raio,
  fundo `ground`, borda `hairline` e placeholder "Enter a web address" em `faint`.
- **Respiração.** Embaixo do campo há um brilho quase invisível (camada de
  sombra com 7 % de `ink`, raio 26) que respira para sempre (ver §6.4).
- **Sobre uma página aberta.** A página continua lá, coberta por um véu de
  `ground` com 74 % de opacidade. Clicar no véu fecha o campo.
- **Autocompletar.** O texto que você digita fica normal. O resto do endereço
  ("git" + "hub.com") já vem escrito e selecionado: continuar digitando
  substitui, e Return aceita.
- **Lista de sugestões.** É um card separado, 8 pt abaixo do campo, também com
  raio 14 e borda `hairline`.
  - Cada linha tem uma coluna fixa de ícone (lupa, favicon ou monograma), o
    título em `ink` e a origem em `muted` ("GitHub", "Google").
  - A primeira linha pode oferecer "Search GitHub", com uma tecla `Tab` em
    keycap à direita.
- **Chip do site.** Um chip com o site pode aparecer à esquerda do texto, dentro
  do campo.
- **Erro.** Com um endereço inválido, a borda fica vermelha (35 %) e o campo
  treme (ver §6.5).

### 5.3 Ajustes (página interna)
- **Coluna à esquerda (~183 pt).** No topo, o título "Settings" e a busca "Find
  a setting ⌘F". Embaixo, uma lista com ícone + rótulo. A seção ativa é uma
  pílula `wash` de 28 pt de altura. Separadores `hairline` dividem os grupos.
- **Conteúdo.** Coluna central de ~512 pt.
  - Os grupos são cards com raio ~10 e borda `hairline`.
  - Cada linha tem um ícone num quadrado arredondado de 24 pt, título e
    descrição, e o controle à direita.
  - Controles usados: toggle iOS-like, popup, botão de cápsula e segmented.
- **Seção MOTION.** Tem "Save power in Low Power Mode" e "Reduce animations".

### 5.4 Coluna lateral (abas no lado)
Observado com Tabs = **Left**:

- **Coluna.** Tem ~208 pt e fica sobre a cor `frame`, quase preta no escuro.
  - Os semáforos ficam no topo e os botões ⟨ ⟩ ↻ ao lado deles (x ≈ 84).
  - Embaixo vêm as abas, uma por linha de 26 pt: favicon de 14 pt + título de
    13 pt. O botão de favorito fica sozinho no rodapé.
  - A aba ativa é uma pílula `wash` com raio ~8 e título em `ink`. As outras
    usam `muted`.
  - No hover, a linha ganha fundo `hover` e mostra um "×" à direita.
  - Por último vem "+ New tab", em `faint`.
- **Card.** A página vira um card que começa colado à coluna, com 8 pt de
  margem no topo, à direita e embaixo e raio ~10. Entre o card e a coluna não
  há nenhuma linha divisória.
- **"Icons only".** É uma coluna de ~44 pt. Os **semáforos ficam empilhados na
  vertical**, e embaixo deles vêm ⟨ ⟩ ↻, também na vertical.
  - Os favicons ficam em quadrados de 30 pt. O ativo ganha um quadrado `wash`
    com raio ~8.
  - Depois vem o "+", e o favorito fica no rodapé. O card começa em x ≈ 44.
- **Opções que só aparecem nesse modo:**
  - Hide the column until the pointer reaches the edge
  - Slide the column in and out
  - ⌘S makes the column icons only
  - The page as a card / Edge to edge (miniaturas)
  - Keep the card in full screen
  - Roomier tabs
  - What's playing in other tabs
- **Coluna recolhida.** Desliza por cima da página quando o ponteiro chega na
  borda (ver §6.3).
- **Grip.** É uma pílula curta no meio da borda, não uma linha.
- **Menu de contexto da aba.** É o menu nativo do macOS. Tem: Move to Group ›,
  Pin, Open in Split View, Move to New Window | Rename, Duplicate, Site
  Information…, Copy Address, Copy as Markdown Link, Copy Clean Link, Open in
  Side Panel, Mute Tab, Put to Sleep | Close Tab, Close Other Tabs, Close Tabs
  Below, Reopen Closed Tab.
- **Popover de favoritos** (botão no rodapé). Fundo `ground`, raio ~10 e borda
  `hairline`. Mostra "No bookmarks yet" em `muted`, depois a linha destacada
  "Add This Page" e "Manage Bookmarks…".

### 5.5 Outros
- **Peek.** Um link aberto a partir de um pin aparece num painel por cima da
  página, que fica escurecida com preto a 22 %. O painel tem sombra de 25 %.
  Enquanto a página não desenha, mostra um "esqueleto" com brilho passando.
- **Aba levantada.** Ao arrastar uma aba para fora da faixa, uma janelinha de
  180 × 30 vai junto com o ponteiro. Ela mostra o ícone e o título, tem raio 9 e
  sombra de 18 %.
- **Glass (opcional).** Fundo da janela com blur do desktop, tint da cor do
  Space e grão. A página continua opaca.
- **Liquid Glass tab (macOS 26, opcional).** A aba ativa vira uma lente de
  vidro que viaja entre as abas. Nesta instalação, a opção não aparece em
  Aparência, nem com as abas no topo nem na coluna. O Glass da janela também
  não aparece; pelo código ele fica na cor do Space, e os Spaces estão
  desligados.
- **Ajustes › Spaces & Pins.** Tem os grupos COMING FROM ARC? ("Set Up Like
  Arc"), SPACES AND PROFILES, PINS e OPENING AND STARTING, com opções como "New
  tab in a floating field", em que ⌘T abre o campo por cima da página, como no
  Arc.

---

## 6. Animações

### 6.1 As curvas base (`Motion`)

| Nome | Curva | Uso |
|---|---|---|
| **glide** | `spring(response 0.34, damping 0.82)` | Algo que **se move** entre dois lugares: a aba ativa trocando de posição, entrar/sair da edição do título de uma aba, rolar até uma aba |
| **settle** | `spring(response 0.30, damping 0.86)` | Algo que **chega ou sai / reordena**: abrir/fechar abas, contagem de pins, soltar um arrasto, o "voltar" depois de pressionar |
| **quick** | `easeOut 0.14 s` | Mudanças pequenas de estado: hover, aparecer/sumir ícones (carregando, som, popups bloqueados), troca do tipo de página, lista de sugestões |
| **give** | escala **0.97** | Aba pressionada afunda (desce em `quick`, volta em `settle`). Opcional: "Tabs give when pressed". |

Para portar, uma spring SwiftUI `response r, damping ζ` equivale a:
`k = (2π / r)²` e `c = 4π·ζ / r` (com massa 1).

- **glide:** k ≈ 341, c ≈ 30,3.
- **settle:** k ≈ 439, c ≈ 36,0.

Na prática, as duas assentam visualmente em ~0,35–0,5 s.

**Modo leve.** Com Low Power Mode ou o Mac quente, e a opção "Save power"
ligada, `glide` e `settle` viram `easeOut 0.16 s`. O motivo é que uma spring
leva ~30 frames para assentar na main thread, e frames perdidos aparecem como
travadas.

**Reduce motion.** Com Reduce Motion ou "Reduce animations":
- toda animação vira `nil`, ou seja, instantânea;
- as durações fixas (`Motion.seconds`) viram 0;
- a respiração do campo para no brilho máximo, parado.

### 6.2 Abas
- **Seleção.** O fundo `wash` da aba ativa desliza até a nova aba com **glide**.
- **Abrir/fechar/reordenar.** A faixa inteira se reacomoda com **settle**,
  disparado por qualquer mudança na lista de IDs de aba.
- **Inserção e remoção.** `scale 0.9` (âncora à esquerda) + `opacity`. Na
  coluna lateral é `scale 0.94`.
- **Arrastar.** As outras abas abrem espaço com **settle**. A aba solta assenta
  com **settle**. Fora da janela vira a "aba levantada" (janela flutuante, sem
  animação de sistema).
- **Pressionar.** Escala 0.97 (ver `give` acima).
- **Indicador de carregamento.** Um anel de stroke `muted` com 70 % de opacidade
  gira no sentido horário, 1 volta a cada **0,85 s**, linear e infinito.
  Aparece/some com fade `quick`.
- **Ícones de estado** (alto-falante, popups bloqueados, tradução): fade `quick`.
- **Renomear aba.** O campo entra e sai com **glide**. Um nome recusado faz a
  mesma tremida do campo de endereço.
- **Liquid Glass.** É uma spring física própria, não uma animação SwiftUI:
  `pull = 320` e `damping = 27`, que dão ~¾ do amortecimento crítico. Por isso
  passa um pouco do alvo e assenta logo. A lente estica na direção em que se
  move. Uma segunda camada, por cima, distorce os títulos só enquanto a lente
  está em movimento.

### 6.3 Coluna lateral
- **Sair por cima da página.** Spring `response 0.14, damping 0.86`. Foi medida
  contra Arc e Dia, que levam ~95 ms até 90 %.
- **Voltar.** Spring `response 0.13, damping 1.0` (crítico, sem rebote).
- **Opção "Slide the column in and out".** `easeOut 0.26 s` para sair e
  `easeIn 0.22 s` para voltar. Com Reduce Motion vira um fade de 0,2 s no lugar.
- **Botões ⟨ ⟩ ↻ mudando entre topo e rodapé.**
  1. A posição antiga sai com `easeIn 0.1 s`.
  2. O topo volta com `easeOut 0.5 s`, depois de 0,1 s de atraso.
  3. O rodapé sobe com `cubic-bezier(0.25, 0.1, 0.25, 1)` em 0,45 s.
- **Redimensionar.** O grip cresce de 28 para 40 pt e escurece com `quick`. Um
  duplo clique volta à largura padrão com **settle**.

### 6.4 Campo de endereço
- **Aparecer/sumir.** `scale 0.97 + opacity`. O véu de 74 % sobre a página
  entra com fade, separado do campo, para as bordas da página não aparecerem.
  Na captura feita durante a abertura de uma aba nova dá para ver o campo já
  nítido sobre a página anterior ainda visível sob o véu.
- **Lista de sugestões.** `scale 0.98` com âncora no **topo** + `opacity`, em
  `quick`. As linhas seguem o que é digitado **sem animação**: com spring, elas
  ficavam atrasadas em relação ao campo.
- **Chip do site.** `scale 0.9` (âncora à esquerda) + `opacity`, em `quick`.
- **Respiração.**
  - Escala de 0,97 a 1,03 e opacidade de 0,65 a 1,0.
  - **2,6 s** para cada lado, `easeInEaseOut`, com autoreverse e infinita.
  - É Core Animation, então não refaz o layout a cada frame.
  - Existe porque "uma coisa que nunca se move parece uma foto de um app".

### 6.5 Tremida de erro (`Shake`)
Translação horizontal `sin(t·6π) · 7 pt · (1 − t)`, com `t` indo de 0 a 1 em
`easeOut 0.5 s`. São três idas e voltas que diminuem até zero. Junto com ela, a
borda fica vermelha a 35 %. A borda volta ao normal com `quick` assim que você
digita.

### 6.6 Página (Stage)
- **Troca de conteúdo** (página, erro, reader, vídeo flutuando): fade `quick`.
- **Capa/snapshot saindo:** fade `easeOut 0.2 s`.
- **Swipe de voltar/avançar** (dois dedos):
  - Um **disco** com seta surge na borda, com `scale 0.85 + opacity` em
    `easeOut 0.16 s`. Ele começa 10 pt para dentro e acompanha o gesto com
    `10 + alcance·0,2`.
  - A opacidade da seta vai de 40 % a 100 % conforme o gesto "arma".
  - Ao confirmar, o disco anda mais 12 pt e some, com `easeOut 0.22 s`.
- **Segurar o swipe.** Vira uma lista do histórico que cresce a partir da borda:
  entra com `scale 0.92`, sai com `scale 0.96`.
  - A linha escolhida troca com `spring(0.22, 0.9)`.
  - Ao soltar, a lista sai com `scale 0.96` e fade em `easeOut 0.26 s`.

### 6.7 Peek
- **Abrir e guardar.** Usa **settle**. O fundo escurece (preto 22 %) com fade.
  O painel entra com `opacity + scale 0.98`.
- **Pinçar para virar aba.** `spring(0.3, 0.92)`. O painel cresce até o tamanho
  da página, ganha os mesmos cantos e vira a aba sem nenhum corte visível.
- **Esqueleto de carregamento.**
  - Só aparece se a página demorar.
  - Entra com fade `quick`.
  - Um brilho atravessa a cada **1,3 s** (`easeInEaseOut`, Core Animation).
  - Com Reduce Motion, fica parado.
- **Selo no pin de origem.** `scale 0.6 + opacity`.

### 6.8 Spaces
- **Trocar de Space** (⌃1–9 ou swipe). As linhas deslizam até o próximo Space
  com `easeOut 0.22 s`.
- **Swipe curto.** Volta para o lugar com **settle**.

### 6.9 Pequenos detalhes
- **Barra de status** (URL do link sob o ponteiro): fade `easeOut 0.12 s`.
- **Linha "at rest"** (aba dormindo): `easeInOut 0.22 s`.
- **Vídeo flutuante (PiP próprio).**
  - Os controles aparecem com fade de 0,16 s quando o ponteiro chega.
  - O snapshot sai com fade de 0,12 s.
  - Recolher e expandir mudam o frame da janela em 0,18 s.
- **Popovers** usam o fundo `ground` em vez do vidro padrão. Assim a seta e o
  corpo ficam da mesma cor.

---

## 7. Resumo para o port (WebView2 / Win32)

- **Curvas:**
  - `glide` = spring k≈341 c≈30.
  - `settle` = spring k≈439 c≈36.
  - `quick` = ease-out de 140 ms.
  - `give` = escala 0.97.
  - Fade padrão de 120–220 ms.
- **Durações fixas:**
  - Respiração: 2,6 s.
  - Anel de carregamento: 0,85 s por volta.
  - Esqueleto: 1,3 s.
  - Tremida: 0,5 s.
- **Entradas e saídas:** sempre *escala levemente menor + fade*, nunca deslize
  longo:

  | Elemento | Escala de entrada |
  |---|---|
  | Campo | 0.97 |
  | Lista de sugestões | 0.98 (âncora no topo) |
  | Abas | 0.9 (âncora à esquerda) |
  | Disco do swipe | 0.85 |

- **Respeitar acessibilidade.** Seguir `SPI_GETCLIENTAREAANIMATION` / "Show
  animations in Windows" como o equivalente de Reduce Motion, e oferecer um
  "Reduce animations" próprio.
- **Animações infinitas** (respiração, anel, esqueleto) devem rodar na camada de
  composição (DirectComposition / CSS `transform` + `opacity`), nunca forçando
  layout a cada frame.
