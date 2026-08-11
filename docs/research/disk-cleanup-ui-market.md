# Pesquisa de UI/UX: alerta e limpeza assistida de disco

Data da pesquisa: 7 de agosto de 2026  
Escopo: macOS, item de menu bar, notificação de pouco espaço, revisão explícita e execução do Mole apenas após confirmação.  
Método: documentação oficial da Apple, documentação/código-fonte dos produtos analisados e inferências de design identificadas como tal. Não houve teste hands-on dos aplicativos proprietários.

> **Decisão posterior, 11 de agosto de 2026:** o fluxo supervisionado foi aprovado, mas a execução in-app permanece bloqueada até existir um contrato estruturado e explicitamente restrito ao usuário. A v1 será distribuída diretamente. O contrato vigente está em [`docs/product/v1.md`](../product/v1.md).

## Conclusão executiva

A direção recomendada para o Statlet é **human-in-the-loop**:

```text
disco acima do limite por 5 minutos
             ↓
badge persistente + uma notificação por episódio
             ↓
clique na notificação ou “Revisar espaço…” no menu
             ↓
janela nativa “Liberar espaço”
             ↓
análise read-only iniciada pela pessoa
             ↓
revisão do plano + confirmação destrutiva explícita
             ↓
execução acompanhada + resultado resumido
```

Nada é removido apenas porque o limite foi ultrapassado. Os 5 minutos permanecem somente como *debounce* do alerta. O episódio é rearmado quando o disco volta abaixo do limite.

Essa direção converge com a Apple e com o mercado:

- notificações devem ser breves, valiosas e não repetitivas;
- uma notificação não oferece contexto suficiente para iniciar uma exclusão;
- o item da menu bar é uma entrada rápida, não a superfície de uma tarefa destrutiva longa;
- ferramentas de limpeza maduras fazem `scan → review → confirm → result`;
- progresso desconhecido deve ser mostrado como indeterminado, sem porcentagem inventada;
- resultado deve mostrar espaço recuperado, estado final e itens ignorados.

Há, porém, um bloqueador técnico anterior ao código: o Mole CLI atual não oferece um contrato seguro e estruturado que preserve a decisão já tomada de limpar **somente o usuário, nunca o sistema**. A UI-alvo pode ser desenhada agora, mas a execução dentro do Statlet não deve ser implementada antes de esse contrato ser resolvido.

## O que esta mudança substitui

- Remover a limpeza automática após cinco minutos acima do limite.
- Remover o consentimento único para automação destrutiva.
- Manter o monitoramento de disco e o badge como opt-in da integração Mole.
- Manter cinco minutos apenas como estabilização antes do alerta.
- Manter uma notificação por episódio e rearmar após voltar abaixo do limite.
- Trocar “Limpar agora” por **“Revisar espaço…”**.
- Exigir confirmação em cada execução.
- Manter histórico local resumido, sem caminhos, com os 30 eventos mais recentes.

## Padrões oficiais da Apple

### 1. Item da menu bar: entrada compacta e dispensável

`NSStatusItem` pode oferecer menu ou ação de clique, mas a Apple pede uso parcimonioso porque o espaço é limitado e o item pode não estar sempre disponível. A documentação também recomenda permitir que a pessoa o oculte. [`NSStatusBar`](https://developer.apple.com/documentation/appkit/nsstatusbar) e [`NSStatusItem`](https://developer.apple.com/documentation/appkit/nsstatusitem).

`MenuBarExtra` confirma dois padrões úteis: o conteúdo pode ser um menu simples ou uma janela parecida com popover quando for mais rico. A própria Apple descreve o extra como acesso a funções comuns mesmo quando o app não está ativo. [`MenuBarExtra`](https://developer.apple.com/documentation/swiftui/menubarextra) e [`MenuBarExtraStyle`](https://developer.apple.com/documentation/swiftui/menubarextrastyle).

**Aplicação ao Statlet:** manter o status item estreito para CPU/RAM e um badge de estado. O menu mostra estado do disco e rotas para a tarefa; a limpeza acontece em uma janela regular. Como fallback de acessibilidade e de ocultação do status item, a janela deve continuar acessível ao abrir o app pelo Finder/Spotlight.

### 2. Notificação: informar e encaminhar, não limpar

A HIG recomenda notificações concisas, relevantes e sem repetições do mesmo fato. Também orienta a evitar uma action que apenas abre o app: tocar na própria notificação já deve revelar o conteúdo relacionado. Ações destrutivas são desencorajadas quando falta contexto. [`Notifications`](https://developer.apple.com/design/human-interface-guidelines/notifications/).

A permissão deve ser solicitada no contexto em que seu benefício fica claro, e o app deve consultar o estado atual porque a pessoa pode mudar a autorização depois. [`Asking permission to use notifications`](https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications).

O clique padrão e ações customizadas chegam ao delegate de `UNUserNotificationCenter`, permitindo abrir diretamente a janela e o estado relacionados ao episódio. [`Handling notifications and notification-related actions`](https://developer.apple.com/documentation/usernotifications/handling-notifications-and-notification-related-actions) e [`UNUserNotificationCenterDelegate`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenterdelegate).

**Aplicação ao Statlet:**

- pedir permissão quando a integração de disco for ativada, não no primeiro launch;
- se a permissão for negada, manter badge, menu e revisão funcionando;
- enviar uma única notificação por episódio;
- tocar no corpo abre “Liberar espaço” no estado daquele episódio;
- não oferecer “Limpar” na notificação;
- não criar botão “Abrir” ou “Revisar”, porque duplicaria o toque padrão;
- não usar nível `timeSensitive` ou `critical` para o limite padrão de 90%; uma notificação normal, sem som próprio, é suficiente;
- se a janela já estiver em primeiro plano, atualizar badge e conteúdo discretamente em vez de duplicar o banner.

Texto recomendado:

```text
Pouco espaço no disco
Macintosh HD está com 92% de uso. Restam 38 GB disponíveis.
```

Evitar caminhos, nomes de arquivos ou outros dados potencialmente sensíveis na notificação.

### 3. Menu: comandos curtos, prioritários e previsíveis

A HIG recomenda colocar itens frequentes/importantes primeiro, agrupar comandos relacionados e evitar menus longos ou submenus desnecessários. [`Menus`](https://developer.apple.com/design/human-interface-guidelines/menus).

Em botões do macOS, uma elipse indica que a ação abre outra janela ou exige entrada adicional. [`Buttons`](https://developer.apple.com/design/human-interface-guidelines/buttons).

Menu recomendado quando a integração está ativa:

```text
Macintosh HD
92% usado · 38 GB disponíveis        (informativo)

Revisar espaço…
Histórico…
Configurações…
──────────────
Sair do Statlet
```

“Revisar espaço…” é preferível a “Limpar agora”: descreve honestamente que análise e decisão ainda virão. O item continua disponível abaixo do limite, permitindo revisão voluntária. Se o Mole estiver ausente ou incompatível, a entrada permanece acionável e abre o estado de correção; um item desabilitado esconderia o caminho de resolução.

### 4. Janela dedicada e confirmação destrutiva

Uma janela macOS deve usar componentes e aparências do sistema, que se adaptam a janela ativa/inativa, Light/Dark Mode e acessibilidade. [`Windows`](https://developer.apple.com/design/human-interface-guidelines/windows).

Uma sheet é adequada a uma tarefa curta e modal ligada à janela pai. [`Sheets`](https://developer.apple.com/design/human-interface-guidelines/sheets). Alertas devem ser reservados para informação crítica e acionável; ações irreversíveis e incomuns merecem confirmação. [`Alerts`](https://developer.apple.com/design/human-interface-guidelines/alerts).

Botões possuem papéis distintos. A Apple orienta a não tornar uma ação destrutiva o botão primário azul, mesmo que seja a escolha mais provável. [`Buttons`](https://developer.apple.com/design/human-interface-guidelines/buttons).

**Aplicação ao Statlet:** usar uma janela regular e persistente para revisão, execução e resultado. O botão “Limpar com Mole…” abre uma sheet final. Nessa sheet, “Cancelar” é a saída segura e “Limpar” usa papel destrutivo/vermelho, não primário.

### 5. Progresso e resultado

A Apple recomenda indicador determinado somente quando o avanço é mensurável e confiável. Para duração desconhecida ou trabalho em background, usar spinner/indicador indeterminado; indicadores desaparecem ao terminar. [`Progress indicators`](https://developer.apple.com/design/human-interface-guidelines/progress-indicators).

**Aplicação ao Statlet:** enquanto o Mole não fornecer progresso estruturado, mostrar spinner, fase textual apenas quando houver sinal confiável e nunca simular uma porcentagem. Fechar a janela não cancela a tarefa. Se houver “Interromper…”, a confirmação precisa explicar que itens já removidos não serão restaurados.

### 6. Acessibilidade

A Apple recomenda cores de sistema, contraste em modos claro/escuro/Increase Contrast e informação transmitida por mais de cor. [`Accessibility`](https://developer.apple.com/design/human-interface-guidelines/accessibility/) e [`Color`](https://developer.apple.com/design/human-interface-guidelines/color).

**Aplicação ao Statlet:** combinar amarelo/vermelho/verde com símbolo e texto (`exclamationmark.triangle`, `checkmark.circle`, `xmark.octagon`), fornecer labels completos para VoiceOver e manter áreas clicáveis compatíveis com o mínimo macOS indicado pela Apple. O badge não pode depender apenas de cor.

### 7. Espaço disponível, purgeable e privacidade

`volumeAvailableCapacityForImportantUsage` representa a capacidade disponível para recursos importantes. [`URLResourceValues.volumeAvailableCapacityForImportantUsage`](https://developer.apple.com/documentation/foundation/urlresourcevalues/volumeavailablecapacityforimportantusage).

A documentação de Required Reason APIs inclui as chaves de capacidade em `NSPrivacyAccessedAPICategoryDiskSpace`. Os motivos candidatos ao Statlet são `85F4.1` para exibir espaço e `E174.1` quando o app verifica pouco espaço para reagir de forma observável e permitir remoção. Os dados derivados não devem sair do dispositivo. A escolha final precisa corresponder exatamente ao comportamento entregue. [`NSPrivacyAccessedAPIType`](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitype).

Há uma inconsistência que precisa ser verificada no toolchain de distribuição: a visão geral de privacy manifests lista Required Reason APIs para iOS, iPadOS, tvOS, visionOS e watchOS, sem citar macOS, enquanto a página da própria key pede declaração em `PrivacyInfo.xcprivacy`. [`Privacy manifest files`](https://developer.apple.com/documentation/bundleresources/privacy-manifest-files) e [`volumeAvailableCapacityForImportantUsageKey`](https://developer.apple.com/documentation/foundation/urlresourcekey/volumeavailablecapacityforimportantusagekey). A escolha conservadora é incluir o manifest no bundle e validar o archive no Xcode/App Store Connect, sem afirmar que a obrigação macOS está resolvida apenas pela documentação atual.

Na UI, usar **“disponível”**, não “livre”, com uma ajuda curta:

> Inclui espaço que o macOS pode recuperar automaticamente.

## Evidência de mercado

| Produto | Entrada/monitoramento | Revisão antes da exclusão | Resultado/padrão relevante |
| --- | --- | --- | --- |
| Mole for Mac | HUD de menu bar e status de disco | A página oficial promete revisar cada item e selecionar, ignorar ou proteger antes da limpeza | Mostra o que executou e por que itens foram ignorados; scan e resultados permanecem locais |
| CleanMyMac | Monitora somente o disco de inicialização; fica amarelo abaixo de 10% disponível; alerta configurável; oferece “Free Up” | Fluxo `Scan → resultados → Review → Run`; detalhes e seleção antecedem remoção | Recalcula storage depois da tarefa e apresenta resumo |
| DaisyDisk | Gauge coloca purgeable na parte disponível para coincidir com Finder/Disk Utility | “Collector” mantém itens intactos até Delete; pode expandir, revisar e remover; diretórios críticos são bloqueados | Exclusão permanente ganha uma janela de cinco segundos para cancelar |
| Stats | Módulos de métricas na menu bar | Não é limpador | Reverteu ao `volumeAvailableCapacityForImportantUsageKey` após concluir que estimar purgeable manualmente era instável |
| Better Resource Monitor | Métricas opcionais, volume principal, cálculo local por total/free reportado pelo sistema | Não é limpador | Não lê nomes/conteúdo de arquivos nem volumes externos; mantém o monitor leve e offline |

Fontes oficiais:

- [Mole for Mac](https://mole.fit/): “review every item”, seleção/proteção, categorias, skips e resultados locais.
- [CleanMyMac: available disk space](https://macpaw.com/support/cleanmymac/knowledgebase/available-disk-space), [Smart Care](https://macpaw.com/support/cleanmymac/knowledgebase/smart-care) e [Space Lens: review and remove](https://macpaw.com/support/cleanmymac/knowledgebase/space-lens-cleanup).
- [DaisyDisk: deleting files](https://daisydiskapp.com/guide/4/en/DeletingFiles/) e [disk overview](https://daisydiskapp.com/guide/4/en/DisksOverview/).
- [Stats v2.12.15](https://github.com/exelban/stats/releases/tag/v2.12.15) e [repositório oficial](https://github.com/exelban/stats).
- [Better Resource Monitor](https://better-resource-monitor.alexpedersen.dev/).

### Convergências

1. Monitoramento barato e local, focado no volume de inicialização.
2. Sinalização rara e contextual, em vez de exclusão silenciosa.
3. Scan iniciado pela pessoa.
4. Revisão do que será removido antes da ação destrutiva.
5. Limites de segurança visíveis e itens ignorados explicados.
6. Resultado mensurável em bytes/GB e estado final do disco.
7. Detalhes sensíveis mantidos localmente.

### Divergências importantes

CleanMyMac e Mole for Mac controlam suas próprias engines; por isso conseguem oferecer seleção granular sincronizada com a execução. DaisyDisk controla o Collector e consegue oferecer um último ponto de recuo. O Statlet dependerá de um CLI externo, hoje sem contrato equivalente. Copiar apenas a aparência dessas ferramentas criaria controles que o backend não consegue honrar.

## Proposta concreta da janela “Liberar espaço”

Uma única janela, sem sidebar na primeira versão. Tamanho inicial aproximado de 680 × 600 pt, redimensionável, com toolbar padrão, título e ação de atualizar quando aplicável. A janela reutiliza a mesma instância se a pessoa clicar várias vezes na notificação ou no menu.

### Estado 1 — antes da análise

```text
┌──────────────────────────────────────────────────────────┐
│ Liberar espaço                                           │
├──────────────────────────────────────────────────────────┤
│ ⚠  Macintosh HD está quase cheio                        │
│                                                          │
│ █████████████████████░░░  92% usado                      │
│ 38 GB disponíveis             Limite configurado: 90%    │
│ ⓘ Inclui espaço recuperável automaticamente pelo macOS   │
│                                                          │
│ O Statlet monitora e avisa. Nada será removido sem sua    │
│ confirmação.                                             │
│                                                          │
│ A análise usa o Mole no seu Mac e não altera arquivos.   │
│                                                          │
│                         [Agora não] [Analisar com Mole]   │
└──────────────────────────────────────────────────────────┘
```

Abrir a janela não inicia scan silenciosamente. “Analisar com Mole” executa somente o modo read-only (`mo clean --dry-run`) depois de a pessoa entender o que ocorrerá.

### Estado 2 — analisando

```text
┌──────────────────────────────────────────────────────────┐
│ Liberar espaço                                           │
├──────────────────────────────────────────────────────────┤
│             ◌  Analisando com Mole…                     │
│                                                          │
│ O Mole está procurando caches, logs e outros itens que   │
│ podem ser recriados. Nenhum arquivo está sendo removido. │
│                                                          │
│                                         [Cancelar análise]│
└──────────────────────────────────────────────────────────┘
```

Spinner indeterminado. A análise é cancelável porque ainda não realizou exclusão.

### Estado 3 — revisão do plano

```text
┌──────────────────────────────────────────────────────────┐
│ Liberar espaço                                           │
├──────────────────────────────────────────────────────────┤
│ Até 12,4 GB podem ser liberados                          │
│ Estimativa do Mole · nenhuma alteração feita             │
│                                                          │
│ ▸ Caches de aplicativos              5,8 GB              │
│ ▸ Ferramentas de desenvolvimento     4,2 GB              │
│ ▸ Navegadores                        2,1 GB              │
│ ▸ Logs e temporários                 0,3 GB              │
│                                                          │
│ ✓ 3 itens protegidos ou em uso serão ignorados           │
│                                                          │
│ A limpeza respeitará a whitelist configurada no Mole.    │
│                                                          │
│                    [Cancelar] [Limpar com Mole…]          │
└──────────────────────────────────────────────────────────┘
```

As linhas são um outline de leitura, não checkboxes. Ao expandir, mostram caminho e tamanho somente nessa janela; caminhos não entram na notificação nem no histórico do Statlet. Categorias com tamanho desconhecido devem declarar “Tamanho não disponível”, nunca `0 B`.

“Limpar com Mole…” abre uma sheet:

```text
Limpar até 12,4 GB?

O Mole removerá os itens mostrados. A ação não pode ser desfeita.
Arquivos protegidos pela whitelist serão ignorados.

[Cancelar]                                      [Limpar]
```

O botão “Limpar” é destrutivo/vermelho. Não pedir digitação ritual ou checkbox de consentimento; a revisão e a sheet já fornecem intenção clara.

### Estado 4 — execução

```text
┌──────────────────────────────────────────────────────────┐
│ Liberar espaço                                           │
├──────────────────────────────────────────────────────────┤
│              ◌  Limpando com Mole…                      │
│                                                          │
│ Isso pode levar alguns minutos. A janela pode ser        │
│ fechada; a limpeza continuará em segundo plano.          │
│                                                          │
│ [Mostrar atividade]                                      │
└──────────────────────────────────────────────────────────┘
```

- spinner indeterminado;
- uma única execução por vez;
- reabrir pelo menu/notificação volta ao processo em andamento;
- fechar a janela não cancela o subprocesso;
- o menu passa a mostrar “Limpeza em andamento…”;
- “Mostrar atividade” exibe stdout/stderr localmente, sem tratá-lo como API estável;
- um eventual “Interromper…” deve explicar em sheet que remoções já concluídas permanecem.

### Estado 5 — resultado

```text
┌──────────────────────────────────────────────────────────┐
│ Liberar espaço                                           │
├──────────────────────────────────────────────────────────┤
│ ✓  Limpeza concluída                                    │
│                                                          │
│                 12,4 GB liberados                        │
│                                                          │
│ Disco       92% → 87%                                    │
│ Disponível  38 GB → 62 GB                                │
│ Duração     38 s                                         │
│                                                          │
│ 2 itens foram ignorados pelo Mole              [Detalhes]│
│                                                          │
│                                               [Concluir] │
└──────────────────────────────────────────────────────────┘
```

Variantes:

- **Concluída com avisos:** houve skips ou espaço liberado, mas o disco continua acima do limite; usar símbolo/texto amarelo e explicar os próximos passos.
- **Sem itens recuperáveis:** nenhum erro; informar que o Mole não encontrou limpeza relevante.
- **Falha:** explicar o motivo e oferecer ação específica (“Instalar Mole…”, “Atualizar Mole…”, “Tentar novamente”). Não reduzir a mensagem a “Erro”.

O Statlet deve medir novamente a capacidade do volume e calcular `antes → depois`; não deve confiar apenas numa frase do subprocesso. O histórico armazena data, duração, uso antes/depois, espaço liberado e estado, sem nomes ou caminhos.

## Estados do badge depois da mudança

O badge continua existindo somente quando a integração Mole estiver ativada:

| Estado | Badge | Persistência |
| --- | --- | --- |
| Dentro do limite | nenhum | — |
| Acima do limite | `!` + amarelo | até voltar abaixo do limite |
| Análise/limpeza iniciada pela pessoa | spinner/indicador azul | enquanto o processo existir |
| Concluída | check + verde | transitório; depois reflete o estado medido do disco |
| Falha/bloqueio de integração | `×` + vermelho | até a condição ser resolvida |

Símbolo/texto sempre acompanha cor. Se a limpeza terminar mas o disco continuar acima do limite, o badge volta a `!` amarelo, não fica verde.

## Bloqueadores encontrados no Mole CLI atual

A análise abaixo foi fixada no commit [`e96df2e16a435c68c0268d81dfe1b78e9845fa83`](https://github.com/tw93/Mole/tree/e96df2e16a435c68c0268d81dfe1b78e9845fa83), consultado em 7 de agosto de 2026. Como o Mole é dependência externa, o comportamento deve ser revalidado contra cada versão compatível.

### 1. `clean --dry-run` não tem JSON documentado

O README documenta JSON para `analyze` e `status`, não para `clean`. [`README`, saída legível por máquina](https://github.com/tw93/Mole/blob/e96df2e16a435c68c0268d81dfe1b78e9845fa83/README.md#L230-L268).

O dry-run produz `~/.config/mole/clean-list.txt`, com headings, caminhos e comentários de tamanho. É útil para uma pessoa, mas não é declarado como schema versionado. [`clean.sh`, geração do preview textual](https://github.com/tw93/Mole/blob/e96df2e16a435c68c0268d81dfe1b78e9845fa83/bin/clean.sh#L417-L483).

**Impacto:** a UI não deve prometer categorias/valores estruturados sem um adapter versionado e testes de compatibilidade. Parsing de ANSI/stdout é ainda mais frágil.

### 2. O CLI não oferece seleção granular reproduzível

`--select`, `--categories` e `--exclude` foram removidos; a proteção persistente é feita por whitelist. [`clean.sh`, parser atual](https://github.com/tw93/Mole/blob/e96df2e16a435c68c0268d81dfe1b78e9845fa83/bin/clean.sh#L2052-L2082).

**Impacto:** checkboxes na tela seriam enganosos. A primeira versão pode revisar categorias e detalhes, mas não selecionar subconjuntos, a menos que o Mole volte a fornecer um contrato executável correspondente.

### 3. “Somente usuário, nunca sudo” não é garantido

No modo não interativo — exatamente como um app chamaria o subprocesso — `mo clean` verifica uma sessão sudo existente. Se encontrar uma credencial cacheada, habilita limpeza de sistema; a limpeza do usuário prossegue automaticamente. O dry-run também inclui a seção de sistema quando há sudo cacheado. [`clean.sh`, modos dry-run e não interativo](https://github.com/tw93/Mole/blob/e96df2e16a435c68c0268d81dfe1b78e9845fa83/bin/clean.sh#L1537-L1579).

O parser atual não oferece `--user-only` ou `--no-sudo`. A variável `MOLE_TEST_NO_AUTH` existe para testes e não é contrato público de produção.

**Impacto:** executar `mo clean` diretamente viola potencialmente a fronteira já aprovada para o Statlet. Não é suficiente “não pedir sudo”; uma sessão de outro terminal pode estar cacheada.

### 4. O plano pode mudar entre análise e execução

Aplicativos podem abrir, caches podem crescer, arquivos podem mudar e o Mole pode revalidar ou ignorar itens entre dry-run e clean. Uma estimativa não deve ser apresentada como garantia.

### Contrato mínimo recomendado ao Mole

Uma integração confiável precisa de algo equivalente a:

```text
mo clean --plan-json --user-only
mo clean --execute-plan <token> --user-only --json-progress
```

O plano deveria incluir `schemaVersion`, versão do Mole, categorias, caminhos, tamanhos conhecidos/desconhecidos, warnings, skips e token/checksum. A execução deveria revalidar o plano e emitir eventos estruturados.

Até isso existir, existem duas opções honestas:

1. **Recomendada:** bloquear a execução in-app e contribuir/negociar o contrato upstream. O Statlet pode monitorar e mostrar a tela de orientação, mas “Limpar” não entra na release.
2. **Fallback:** após a revisão, abrir o Mole oficial/Terminal e deixar a pessoa conduzir a limpeza interativa. Isso preserva contexto e confirmação, mas quebra a experiência contínua e não garante a fronteira “sem sudo” se a pessoa aceitar ampliar o escopo.

Não usar parsing de prompt/ANSI nem uma variável de teste como contrato de produção.

## Distribuição e sandbox

A Mac App Store exige App Sandbox. [`App Sandbox`](https://developer.apple.com/documentation/security/app-sandbox). O Sandbox restringe acesso a recursos por entitlement, e a Apple documenta o caminho suportado para executar um helper **embutido** no app. O Statlet, por outro lado, pretende localizar e executar um `mo` instalado externamente e deixá-lo varrer áreas do diretório do usuário.

Isso não prova por si só que toda integração externa seja impossível, mas constitui risco arquitetural alto para uma distribuição sandboxed. O fluxo precisa ser prototipado sob App Sandbox antes de assumir compatibilidade com a Mac App Store. A distribuição direta assinada e notarizada via Developer ID é a candidata mais compatível com o contrato atual do Mole.

A decisão foi tomada em 11 de agosto de 2026: a v1 terá distribuição direta, assinada e notarizada. A Mac App Store fica condicionada a uma futura arquitetura compatível com App Sandbox.

## Recomendação final

O desenho da janela e o fluxo de notificação foram adotados como direção de produto. Situação dos gates:

1. **Aprovado:** o Statlet apenas alerta; nunca limpa ao cruzar o limite.
2. **Aprovado:** purgeable conta como disponível e a interface usa “disponível”.
3. **Aprovado:** distribuição direta na v1.
4. **Bloqueado externamente:** obter do Mole um modo estruturado e explicitamente `user-only` antes de habilitar “Limpar” no Statlet.
5. **Pendente de protótipo:** testar o fluxo completo com VoiceOver, Increase Contrast, Light/Dark Mode, notificações negadas, Mole ausente/incompatível, sudo cacheado, sleep/wake e reentrada por múltiplos cliques.

Sem o gate 4, a UI de execução pareceria segura, mas o backend poderia ampliar o escopo silenciosamente. Esse é o principal risco identificado pela pesquisa.
