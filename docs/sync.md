# Sync por pasta (issue #7)

Desenho do sync entre PCs. A base é o upstream (`Sources/Search/Sync.swift` e
`SyncMerge.swift`, de 17/11/2026), adaptado ao Windows. Ainda não há código:
a crate `rust/crates/sync` só começa depois que este documento for aprovado.

## A ideia em uma frase

O app escreve arquivos pequenos numa pasta que o usuário escolhe, por exemplo
no OneDrive, no Dropbox ou num disco compartilhado. Quem leva a pasta de um PC
para o outro é o programa de nuvem do usuário. Não há conta, servidor nem nada
nosso no meio.

## Desligado por padrão

- Sem `sync.json` na pasta de dados, o sync está desligado.
- Desligado, ele **não cria nada**:
  - nenhum timer, thread ou observador;
  - nenhum arquivo;
  - nem o id do PC, que só nasce ao ligar.
- Liga em `search://settings`, no item "Sincronizar com seus outros PCs":
  - o seletor de pasta nativo abre (`IFileOpenDialog` com `FOS_PICKFOLDERS`);
  - ele começa em `%OneDrive%`, se existir.
- Depois de ligado, `sync.json` guarda `{ "folder": "...", "device": "<uuid>" }`.
  O `prefs.json` não muda.
- Nada sai do PC por conta do app. O app só lê e escreve numa pasta local.
  Se a pasta vai para a nuvem, isso é escolha do usuário.

### Desligar

Ao desligar, o app:

1. apaga os arquivos deste PC na pasta compartilhada;
2. apaga os arquivos de estado locais (`sync-<escopo>.json`);
3. apaga o `sync.json`.

O que está no navegador fica como está. Religar começa como um PC que acaba de
entrar.

## O que sincroniza

| Sim | Não |
|---|---|
| Favoritos (por perfil) | Histórico |
| Spaces: nome, cor, ícone, ordem, se tem logins próprios | Logins, cookies e dados de site do Space |
| Fixados de cada Space: letra, nome, página, ordem | Abas abertas não fixadas, grupos, janelas privadas |
| Ajustes da lista abaixo | Pasta de downloads, tamanho de janela, extensões |
| | **Senhas** (decisão abaixo) |

Os ajustes que sincronizam são as chaves do `Prefs` que dizem como o app se
comporta, e não o que é deste PC:

- `engine`, `shield`, `sleep`, `tabs`;
- `close_goes_back`, `reader_look`;
- `split_view`, `float_away`, `peek`, `peek_with`.

`custom_engine` **não** sincroniza. Quem pudesse escrever na pasta mandaria
todas as buscas para um endereço escolhido por ele. O upstream sincroniza esse
ajuste; aqui ele é de cada PC.

Elas ficam numa lista fixa na crate. Um ajuste novo só entra se for
acrescentado à lista de propósito.

**Perfis.** Cada perfil sincroniza à parte e nunca se mistura com outro.

- Pessoal (`PERSONAL_ID`) é o mesmo perfil em todo PC.
- Um outro perfil só casa com o de mesmo id em outro PC. Dois perfis criados
  separadamente em cada PC ficam separados, como no upstream.
- A lista de perfis em si não sincroniza.

**Fixados compartilhados** (os que todo Space mostra) ainda não existem no
port. Quando existirem, entram com a chave `pin:shared:…`, como no upstream.

## Senhas: a decisão

**Decidido em 21/11/2026: não sincronizar.** O upstream também não sincroniza
("Never your history, passwords, cookies or open tabs").

1. **Sai do PC.** As senhas estão no Gerenciador de Credenciais, protegidas
   pela conta do Windows. Sincronizar põe uma cópia delas num arquivo que vai
   para a nuvem de terceiros.
2. **Precisaria de cripto própria.**
   - O DPAPI só abre no mesmo usuário do mesmo PC, então não serve entre PCs.
   - Seria preciso uma senha-mestra digitada em cada PC, um KDF (Argon2 ou
     scrypt) e uma cifra autenticada (AES-GCM ou ChaCha20-Poly1305).
   - Isso traz crates novas, exe maior e uma superfície de ataque que hoje não
     existe.
   - Uma senha-mestra esquecida perde tudo. Uma fraca expõe tudo a quem tiver
     acesso à pasta.
3. **Já existe um caminho manual.** A Fase 3 importa CSV de senhas. Levar as
   senhas de um PC para outro continua possível sem nada sempre ligado.

Se a decisão for "sim", vira uma issue à parte, com revisão de segurança
própria. A chave nunca fica na pasta, a crate usa cripto auditada, e o recurso
é desligado por padrão, separado do resto do sync.

## Arquivos na pasta escolhida

```
<pasta escolhida>/
  SearchWin Sync/            ← muda junto com o nome do app
    app/
      <device>.json          ← ajustes
    profile-<id do perfil>/
      <device>.json          ← Spaces, fixados e favoritos desse perfil
```

- **Um arquivo por PC por escopo.** Só aquele PC escreve nele, então dois PCs
  nunca disputam o mesmo arquivo. Por isso quase não aparece cópia de
  conflito do OneDrive ou do Dropbox.
- **Só lemos arquivos cujo nome é `<uuid>.json`** e cujo `device` interno bate
  com o nome. Cópias de conflito (`x-PCNOME.json`, `x (conflicted copy).json`),
  temporários e lixo são ignorados.
- **Escrita atômica.** O app grava `.<device>.tmp` e renomeia por cima. Ele só
  escreve quando o conteúdo mudou.
- **Estado local** em `%LOCALAPPDATA%\SearchWin-rs\sync-<escopo>.json`: o que
  este PC combinou com a pasta da última vez. Fica fora da pasta compartilhada.

### Formato (`format: 1`, igual ao do upstream)

```json
{
  "format": 1,
  "device": "6F1C…",
  "name": "DESKTOP-CASA",
  "scope": "profile-00000000-0000-0000-0000-0000000000A1",
  "written": 1795000000.0,
  "items": {
    "bm:<uuid>":        { "v": "{\"title\":\"Rust\",\"url\":\"https://rust-lang.org/\"}", "t": 1795000000.0, "d": "6F1C…" },
    "kids:root":        { "v": "[\"<uuid>\",…]", "t": …, "d": … },
    "space:<uuid>":     { "v": "{\"colour\":2,\"icon\":null,\"name\":\"Trabalho\",\"sharesSignIns\":false}", … },
    "order:spaces":     { "v": "[…]", … },
    "pin:<space>:<id>": { "v": "{\"home\":\"https://…\",\"letter\":\"G\",\"name\":null}", … },
    "order:pins:<space>": { … },
    "title:pin:<id>":   { … },
    "bm:<uuid apagado>": { "v": null, "t": …, "d": … }
  }
}
```

- Cada item é um texto JSON pequeno com as chaves ordenadas, para que o mesmo
  valor dê sempre o mesmo texto e "mudou" signifique que mudou de verdade.
- `t` é o carimbo em segundos e `d` é o PC que fez a mudança.
- `v: null` é uma lápide: a exclusão viaja como qualquer mudança.
- No escopo `app`, os itens são `set:<chave>` com o valor de `Prefs::to_value`.
- `name` vem de `%COMPUTERNAME%` e só serve para a tela de ajustes dizer quem
  foi visto.
- Arquivo com outro `format`, com o escopo errado, danificado ou com mais de
  4 MB é pulado e nunca lido pela metade.

### Ids

- Favoritos e Spaces já têm ids no formato UUID, compatíveis com o upstream.
- **Fixados** ganharam `pin_id` na `Tab` e na `Entry` da sessão, com
  `serde(default)`.
  - O id nasce na primeira passada do sync que vê o fixado, e a fileira é
    regravada.
  - Desafixar apaga o id.
  - Sessões antigas continuam abrindo.
- **Fixados de outro PC e fileiras guardadas:**
  - Um fixado que chega de outro PC entra dormindo, na ordem combinada.
  - Um fixado removido em outro PC é só **desafixado** aqui: a aba continua
    aberta, para que nada se feche debaixo da sua mão. O upstream fecha a aba.
  - Abas privadas nunca são lidas nem tocadas.
  - Um Space aceita no máximo 64 fixados novos vindos da pasta. Os que já
    existem aqui nunca são desafixados por causa desse limite.
  - Se todos os fixados (5 ou mais) de um Space somem de uma vez, a mudança
    não é aplicada. A regra vale Space por Space.
  - Se o arquivo de sessão de uma fileira guardada existe mas não dá para
    ler (outro programa segurando), a passada não roda. Ler esse arquivo como
    vazio faria a fileira ser regravada vazia.
  - Para ler os fixados de cada Space, o sync carrega a fileira guardada
    dele, do mesmo jeito que ir até o Space carregaria. Isso custa só a lista
    de abas, e a fileira é regravada quando muda.

## A mescla, por registro

É a mesma do upstream. Fica numa parte pura da crate, sem arquivo nem janela,
para que cada regra tenha teste próprio.

Uma passada em um PC:

1. **Carimbar.** O app compara os itens como estão agora com a base (o que
   ele combinou da última vez).
   - Cada diferença é uma edição feita aqui. Ela recebe o carimbo
     `max(agora, maior carimbo visto + 0,001)`. Assim, um PC com o relógio
     atrasado ainda vence um valor que ele já viu.
   - Um item que estava na base e sumiu vira lápide.
2. **Mesclar.** O app junta os itens deste PC com os arquivos dos outros, item
   a item. A ordem de desempate é:
   1. o carimbo maior;
   2. o maior id de PC;
   3. lápide antes de valor;
   4. o texto maior.

   Essa ordem é total, então todo PC chega à mesma resposta, em qualquer ordem
   de leitura.
3. **Aplicar e gravar.** O que mudou é aplicado aqui. O resultado vira a nova
   base e o arquivo deste PC.

### Listas e árvores

- **Ordem** (Spaces, fixados, filhos de uma pasta de favoritos): é um item
  próprio com a lista de ids.
  - Ao aplicar, entram primeiro os ids na ordem combinada, depois os que
    faltam na ordem local, e por último os demais por id.
  - Um item criado num PC enquanto outro reordenava vai para o fim, sem se
    perder.
- **Favoritos órfãos**, cuja pasta foi apagada num PC enquanto eles eram
  criados em outro, vão para o fim da raiz em vez de sumir.
- **Um fixado em dois lugares** (compartilhado e de um Space) fica como
  compartilhado.

## Conflitos e exclusões

- **Edição × edição** no mesmo item: vence o carimbo mais novo. Edições em
  itens diferentes, como renomear um favorito aqui e movê-lo lá, convivem,
  porque o título e a posição são itens separados.
- **Edição × exclusão:** vence o carimbo mais novo. Se a exclusão for mais
  nova, o item some. Se a edição for mais nova, ele volta.
- **Lápides** duram 365 dias e depois são esquecidas. Um PC que fique mais de
  um ano sem sincronizar pode trazer de volta algo apagado nesse tempo, e
  nada pior que isso.
- **Proteção contra perda em massa.** Se uma categoria inteira some de uma
  vez, ela **não** vira lápide e volta da pasta. As categorias são favoritos,
  fixados e Spaces, e a regra vale quando havia 5 ou mais itens. Arquivo que
  não leu ou navegador zerado é muito mais provável do que uma escolha do
  usuário.
  - A regra vale nos dois sentidos. Se o resultado da mescla esvazia uma
    categoria que aqui tem 5 ou mais itens, ela não é aplicada, e a próxima
    passada carimba de novo o que está aqui.
  - Antes de regravar os favoritos por causa de outro PC, o app guarda a
    versão anterior em `bookmarks.json.before-sync`.
- **A base só é gravada depois de aplicar.** Se a aplicação falha (arquivo
  travado, disco cheio), a base continua a antiga. Assim, os valores velhos
  daqui não parecem edições novas na passada seguinte.
- **Carimbo do futuro:** uma entrada com carimbo mais de 1 dia à frente do
  relógio é descartada. Se não fosse, ela venceria toda mescla para sempre.
- **PC entrando** (sem base e com outros PCs já na pasta): os itens dele
  recebem carimbo 0.
  - Onde os dois lados têm o mesmo item, vale o que já está na pasta.
  - O que só este PC tem é preservado.
  - Ao ligar com uma pasta, o PC sempre entra como novo, mesmo que já tenha
    usado aquela pasta antes.
- **Pasta ausente** (disco desligado, OneDrive fechado): não é motivo para
  achar que tudo foi apagado. A passada não roda e os ajustes mostram "A pasta
  não está lá".
- **Item que esta versão não entende**, de uma versão mais nova ou danificado:
  segue adiante intacto e nunca vira lápide. O favorito ou Space daqui
  correspondente continua como está.
- **Space apagado em outro PC:** é tratado como "Apagar Space" aqui. As abas e
  os fixados vão embora. Se o Space tinha logins próprios, eles também são
  apagados. O Space inicial do perfil nunca é removido.
- **"Tem logins próprios"** vale como ficou combinado na primeira vez e nunca
  muda depois. Trocar o armazenamento por baixo de abas abertas quebraria as
  abas.

## Quando roda

- **Leveza.** Nada de thread própria nem observador de pasta na primeira
  versão.
- Uma passada roda:
  - 2 s depois de uma mudança local em favorito, Space, fixado ou ajuste
    (mudanças seguidas se juntam numa só passada);
  - a cada 30 s, com `SetTimer`, só enquanto estiver ligado;
  - quando o app volta ao primeiro plano (`WM_ACTIVATEAPP`), se a última
    passada foi há mais de 10 s.
- A passada roda na thread da janela. Uma pasta de rede lenta pode travar a
  janela por um instante. Isso só acontece com o sync ligado; uma thread
  própria fica para quando fizer falta.
- **Custo de uma passada:** ler alguns arquivos pequenos e escrever o deste PC
  só se mudou.
- **Observar a pasta** com `ReadDirectoryChangesW` fica para depois, se 30 s
  parecer lento na prática.

### OneDrive "arquivos sob demanda"

Um arquivo que está só na nuvem é um marcador de posição. Ler esse arquivo
dispara o download e travaria a janela.

- Antes de ler, o app olha `GetFileAttributesW`. Arquivos com
  `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` ou `FILE_ATTRIBUTE_OFFLINE` são
  pulados nesta passada.
- Os ajustes sugerem marcar a pasta como "Sempre manter neste dispositivo".

## Segurança: a pasta é entrada não confiável

Quem tiver acesso à pasta, como outra conta da nuvem com a pasta
compartilhada, pode escrever arquivos ali. Por isso:

- **Favoritos e fixados:** a URL passa pela mesma checagem de quando se
  adiciona um favorito. Só valem `http`, `https` e `file`, os mesmos que o
  app guarda localmente. `javascript:`, `data:` e similares são descartados.
  - Um item recusado nunca é aplicado e também nunca vira lápide: no PC de
    origem, ele continua intacto.
- **Ajustes:** entram por `Prefs::set`, que já recusa valores fora do tipo.
  `custom_engine` nunca vem da pasta.
- **Textos** (nomes de Space, títulos, nome do PC) têm tamanho máximo, e
  caracteres de controle e de direção (U+202E e afins) são removidos.
- **Árvore de favoritos:** a profundidade máxima é 64 pastas. O que passa
  disso vai para a raiz, sem estouro de pilha.
- **Nada da pasta vira script nem chega à ponte `search://`** como comando:
  são só dados.
- **Limites:**
  - 4 MB por arquivo e 32 outros PCs por escopo;
  - só arquivos `<uuid>.json` nas pastas de escopo conhecidas;
  - nenhum caminho é montado a partir do conteúdo.
- **Fica de fora:** uma junção (reparse point) que alguém ponha no lugar da
  pasta de escopo seria seguida. Isso exige acesso de escrita à pasta e só
  permite gravar o arquivo deste PC em outro lugar.

## Estado (23/11/2026)

- **Feito:** as três etapas (ajustes, favoritos, Spaces e fixados).
  - Crate `rust/crates/sync`, com 25 testes.
  - `Spaces::adopt` na crate `spaces`, com 1 teste.
  - `pin_defs` e `adopt_pins` na crate `tabs`, com 3 testes.
  - `rust/src/sync.rs` e a seção "Sincronizar" em `search://settings`.
  - O workspace tem 350 testes, todos passando, e o clippy está limpo.
- **Bench:** 24 verificações passaram, com `SEARCHWIN_DATA` e `SEARCHWIN_SYNC`
  em pastas temporárias. Cobrem:
  - desligado não cria nada;
  - ligar;
  - fixado antigo ganhando id e saindo;
  - receber ajuste, favorito, Space e fixado (na ordem combinada);
  - recusar `javascript:` e cópia de conflito;
  - exclusão vinda de outro PC, com o fixado desafixado e a aba mantida;
  - `pin_id` gravado na sessão;
  - edição local saindo;
  - desligar.
- **Revisão:** feita por agentes separados de Rust e de segurança, e os
  achados estão corrigidos acima.
- **Exe:** 1305 KB → 1434 KB (+129 KB), mais que o previsto. Vale procurar
  onde encolher. A memória e a primeira página não foram medidas; com o sync
  desligado, só é lido um arquivo que não existe.
- **Fica de fora:** fixados compartilhados (ainda não existem no port).

## Implementação, depois da aprovação

**Crate `rust/crates/sync`**, sem dependência nova (já existe `serde_json`; o
UUID vem do mesmo gerador de `spaces`). A parte pura é testada sozinha:

- `merge.rs`: `Entry`, `File`, `wins`, `merge`, `stamp`, `pass`, `prune`,
  `ordered`, `decode` e `encode`;
- `items.rs`: favoritos, Spaces, fixados e ajustes ↔ itens, com a validação
  da seção de segurança.

**Lado do app**, em `rust/src/sync.rs`:

- `sync.json`;
- a passada;
- o timer;
- aplicar os itens nas janelas abertas;
- a linha de status nos ajustes.

**Ordem de entrega**, cada etapa testável:

1. ajustes e favoritos;
2. Spaces;
3. fixados, que precisam de `pin_id` na `tabs` e de aplicar o resultado em
   abas vivas.

**Testes:**

- unidade na crate: cada regra acima, inclusive PC atrasado, lápide contra
  edição, categoria inteira sumindo, órfãos e arquivo estranho;
- bench com `SEARCHWIN_DATA` temporário e um gancho novo, `SEARCHWIN_SYNC`
  (pasta de sync sem o diálogo, só vale com `SEARCHWIN_BENCH=1`). O "outro
  PC" é simulado escrevendo o arquivo dele direto na pasta, com o `encode` da
  crate.

**Para testar à mão:**

- o seletor de pasta;
- dois PCs reais com o OneDrive;
- arquivos sob demanda.

**Custo esperado:** exe com algumas dezenas de KB a mais. Memória e primeira
página iguais com o sync desligado. Os números vão para o PLANO.md ao fechar.

**Fora desta issue:**

- conversar com o Search do Mac na mesma pasta (os formatos são parecidos,
  mas as chaves de ajuste e o modelo de fixados diferem);
- criar perfis entre PCs;
- observar a pasta em tempo real;
- senhas (ver acima).
