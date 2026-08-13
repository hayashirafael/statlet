# Pesquisa de produto: CPU e RAM no Statlet

Data da pesquisa: 13 de agosto de 2026
Escopo: apps macOS de menu bar, monitores nativos de desktop e monitores de terminal que exibem CPU/RAM.
Método: documentação, código-fonte e canais oficiais. As comparações transversais e propostas são explicitamente tratadas como inferências/recomendações; fatos de produto são acompanhados por fontes. Não houve teste hands-on dos apps proprietários nem benchmark dos concorrentes.

## Conclusão executiva

A direção recomendada é **preservar o indicador compacto atual e aprofundar a janela sob demanda “Uso do sistema”**:

```text
menu bar: CPU + RAM em duas linhas, sem nova métrica permanente
                         ↓ clique
Uso do sistema: CPU | RAM | GPU
                         ↓
resumo global + histórico curto + composição
                         ↓
processos somente quando agregarem diagnóstico e o custo estiver medido
                         ↓
“Abrir Monitor de Atividade” para investigação e ações avançadas
```

Essa hierarquia é a convergência mais forte da amostra. iStat Menus, Stats, MenuMeters e Usage mantêm um sinal pequeno na barra e levam histórico, decomposição e processos para uma superfície aberta sob demanda. Activity Monitor, Task Manager e GNOME System Monitor também separam o estado global da lista de processos. O btop mostra ambos simultaneamente, mas em uma superfície de investigação ativa, não em um indicador passivo.

Para o Statlet, copiar a amplitude dos concorrentes seria um erro de produto. O melhor recorte é:

1. corrigir e medir a coleta existente;
2. tornar explícito no seam o sample de CPU/RAM que o cache já compartilha por ciclo;
3. adicionar uma seção CPU curada à janela já existente;
4. manter per-core, alertas e CPU por processo para etapas posteriores, condicionadas a evidência.

CPU na janela exige **emenda ou novo ADR**: o ADR 0004 aceito limita explicitamente “Uso do sistema” a RAM e GPU. Este documento recomenda a mudança; não a trata como já aprovada.

## Como a amostra foi escolhida

“Mais usados” não tem ranking público comparável entre produtos pagos, App Store e open source. A seleção usa sinais observáveis de adoção, longevidade e manutenção; eles não equivalem a usuários ativos ou market share.

| Produto | Sinal observável em 2026-08-13 | Leitura responsável |
| --- | --- | --- |
| **Stats** | 41.174 stars e 1.471 forks na API oficial do GitHub; release `v3.0.11` em 9 de agosto de 2026. [Repositório/API](https://api.github.com/repos/exelban/stats), [release](https://github.com/exelban/stats/releases/tag/v3.0.11) | Forte interesse open source e manutenção recente; stars não são instalações. |
| **Usage** | O site declarava média global 4,7 com mais de 25 mil avaliações; a página US da App Store mostrava 2,5 mil avaliações e 4,7/5. [Site](https://usage.pro/), [App Store](https://apps.apple.com/us/app/usage-device-monitor/id1561788435?platform=mac) | Alcance relevante, mas números de canais/plataformas diferentes não devem ser somados. |
| **iStat Menus** | Produto lançado em 2007 e atualizado continuamente; versão 7.3 publicada em maio de 2026. [Histórico](https://bjango.com/articles/twodecades/), [versões](https://bjango.com/mac/istatmenus/versionhistory/) | Longevidade e manutenção sustentam sua relevância; avaliações de um storefront não medem a base total. |
| **MenuMeters** | 3.073 stars e 231 forks; release pública mais recente de 2021. [Repositório/API](https://api.github.com/repos/yujitach/MenuMeters), [release](https://github.com/yujitach/MenuMeters/releases/tag/2.1.6.1) | Referência histórica valiosa para compactação, mas menos atual. O próprio README recomenda alternativas modernas. |

Foram incluídos ainda **Activity Monitor**, **Windows Task Manager**, **GNOME System Monitor**, **KDE Plasma System Monitor** e **btop** porque suas fontes primárias deixam claros padrões de diagnóstico, processos, memória, cadência e acessibilidade que não dependem do formato menu bar.

## Comparação dos apps macOS de menu bar

| App | Visão rápida | CPU no detalhe | RAM no detalhe | Cadência/custo | Ideia útil para o Statlet |
| --- | --- | --- | --- | --- | --- |
| **iStat Menus 7** | Itens configuráveis, modo combinado e espaçamento compacto | Total, User/System/Idle, histórico, load average, E/P cores, por core e top apps | App/Wired/Compressed/Free, pressão, swap e top apps | Frequência global configurável; histórico agrega médias; o fabricante associa menor frequência a menor custo | Seções reordenáveis no detalhe e regras temporizadas são boas referências, mas a superfície total é ampla demais para o Statlet. [Produto](https://bjango.com/mac/istatmenus/), [menus](https://bjango.com/help/istatmenus7/menus/), [histórico](https://bjango.com/help/istatmenus7/historygraphs/) |
| **Stats** | Módulos com mini, linha, barras, pizza e gauge | Total, User/System/Idle, histórico, cores/clusters, load average e top processes | Composição, pressão Normal/Warning/Critical, swap, histórico e processos | CPU/RAM e processos usam 1 s por padrão; opções de 1–60 s. O README reconhece custos diferentes entre módulos | É a referência mais auditável para separação de resumo, histórico, detalhes e processos; o Statlet deve copiar a hierarquia, não a quantidade de opções. [README](https://github.com/exelban/stats), [CPU](https://github.com/exelban/stats/blob/327eb11160e529cd4ca4e1c82007154941550c2e/Modules/CPU/popup.swift#L135-L321), [RAM](https://github.com/exelban/stats/blob/327eb11160e529cd4ca4e1c82007154941550c2e/Modules/RAM/popup.swift#L137-L267) |
| **MenuMeters** | Mostradores austeros e combináveis | System/User, percentual/gráfico e até 25 processos | Active/Wired/Inactive/Free, compressed, pressão, swap e paginação | CPU: 0,5–10 s, padrão 1 s; RAM: 1–60 s, padrão 10 s | Cadências diferentes por volatilidade/custo são uma ideia forte; a apresentação é útil como limite de densidade. [README](https://github.com/yujitach/MenuMeters), [CPU](https://github.com/yujitach/MenuMeters/blob/e91b746debd15777012968a4d247a074d10402f6/Common/MenuMeterCPU.h#L79-L112), [RAM](https://github.com/yujitach/MenuMeters/blob/e91b746debd15777012968a4d247a074d10402f6/Common/MenuMeterMem.h#L74-L99) |
| **Usage** | Mais de 40 componentes, widgets e app detalhado | Utilização, histórico, temperatura e processos | RAM, pressão, swap e processos | Refresh configurável, sem valores/defaults públicos nas páginas consultadas; sensores e processos podem exigir Helper | Visual moderno e continuidade entre superfícies são bons sinais; mais de 40 componentes e dependência de helper não combinam com o recorte leve do Statlet. [Site](https://usage.pro/), [App Store](https://apps.apple.com/us/app/usage-device-monitor/id1561788435?platform=mac), [Helper](https://usage.pro/mac/helper) |

### Convergências observadas — inferências

- **Glance + drill-down.** Os quatro preservam uma leitura imediata e movem explicação para menu, popover ou app.
- **CPU total precisa de contexto no detalhe.** User/System/Idle, histórico e processos explicam a carga sem ocupar permanentemente a barra.
- **RAM usada isoladamente é um diagnóstico fraco.** Pressão, compressão, cache e swap explicam por que um percentual alto pode ou não ser problema.
- **Top processes responde “quem está causando?”.** É útil depois do sinal global, não como conteúdo da barra.
- **Cadência é parte do contrato de custo.** Produtos maduros expõem ou documentam o trade-off entre responsividade e overhead.
- **A customização também resolve espaço.** Modos compactos existem porque a menu bar é um recurso escasso; o Statlet já toma a decisão mais simples ao usar um único item.

## O que os monitores de sistema acrescentam

| Produto | Padrão relevante | Limite de transferência |
| --- | --- | --- |
| **Activity Monitor** | CPU System/User/Idle; pesquisa e ordenação de processos; Memory Pressure verde/amarelo/vermelho; composição e swap; atualização padrão de 5 s | É uma janela de diagnóstico completa. O Statlet deve oferecer atalho para ela, não reproduzi-la. [CPU](https://support.apple.com/guide/activity-monitor/actmntr43452/mac), [memória](https://support.apple.com/guide/activity-monitor/view-memory-usage-actmntr1004/mac), [cadência](https://support.apple.com/guide/activity-monitor/actmntr2224/mac) |
| **Windows Task Manager** | Separa “há pressão?”, “quem contribui?” e “qual ação tomar?”; gráficos e tabelas têm superfícies distintas; documentação específica para leitor de tela | Ações como End Task aumentariam risco e escopo. Abrir a ferramenta nativa é mais seguro. [Task Manager](https://learn.microsoft.com/en-us/troubleshoot/windows-server/support-tools/support-tools-task-manager), [acessibilidade](https://support.microsoft.com/en-us/accessibility/windows/use-a-screen-reader-to-navigate-windows-support-tools) |
| **GNOME System Monitor** | Histórico global + ranking de processos; documenta que intervalos menores aumentam o uso de CPU do próprio monitor | Métricas convencionais são claras, mas não há equivalente documentado à pressão de memória composta da Apple. [Ajuda](https://help.gnome.org/gnome-system-monitor/), [cadência](https://help.gnome.org/users/gnome-system-monitor/3.22/process-update-speed.html.en) |
| **KDE Plasma System Monitor** | Modelo extensível de sensores, páginas e gráficos personalizáveis | É o extremo “dashboard configurável”, oposto ao produto compacto e curado definido pelo Statlet. [Produto](https://apps.kde.org/plasma-systemmonitor/), [visões](https://kde.org/announcements/plasma/5/5.21.0/) |
| **btop** | Painéis densos, presets, processo selecionado, filtros e atualização configurável; recomenda `update_ms` de 2000 ms ou mais | A densidade funciona numa sessão investigativa de terminal, não numa superfície passiva. [README](https://github.com/aristocratos/btop#readme) |

### Lições transferíveis — inferências/recomendações

1. **Resumo, histórico e processos são camadas distintas.** O indicador responde “como está?”; a janela responde “o que aconteceu?”; processos respondem “quem contribuiu?”.
2. **Pressão é o sinal principal de saúde da RAM.** Bytes e composição permanecem como explicação. Cor nunca deve ser o único canal.
3. **Histórico curto em memória é suficiente para distinguir pico de tendência.** Não há necessidade de persistir séries ou processos para o diagnóstico imediato.
4. **Atualizar mais rápido tem custo.** Apple e GNOME avisam isso explicitamente; btop recomenda pelo menos 2 s para amostras melhores.
5. **Sinalização não é alerta.** Cor/heatmap dentro da janela não justifica notificações de CPU/RAM. Alertas exigiriam limiar, duração, cooldown, sono e semântica de recuperação.
6. **Acessibilidade precisa ser contrato.** Gráficos devem ter resumo textual, estado não pode depender só de cor e tabelas precisam de foco/ordenação previsíveis.

## Estado atual do Statlet

### O que já está bem alinhado

- O domínio define o **indicador compacto** como CPU e RAM em duas linhas num único item, evitando widget/dashboard (`CONTEXT.md`).
- O indicador mostra CPU e RAM como texto percentual, tem largura estável para 0–100%, tooltip e rótulo acessível (`src/indicator.rs`, `src/macos/renderer.rs`).
- `MacSampler` retém uma instância de `sysinfo::System` e guarda o sample por `SamplingCycle`, evitando uma segunda leitura do SO no mesmo ciclo (`src/macos/sampler.rs`).
- A cadência compacta aceita 1–60 s e usa 2 s por padrão (`src/indicator_preferences.rs`).
- “Uso do sistema” já tem uma sessão profunda e testável, coleta somente quando visível, histórico de 150 pontos/5 minutos, lacunas sem interpolação e processos RAM a cada 6 s (`src/system_usage.rs`, `tests/system_usage.rs`).
- A RAM detalhada já separa Apps, Reservada, Comprimida, Disponível, Cache recuperável e Swap; a fórmula visível exclui cache recuperável, em linha com o modelo conceitual documentado pela Apple (`src/metrics.rs`).

### Lacunas e riscos encontrados

1. **Primeira amostra de CPU potencialmente enganosa.** `sysinfo` calcula CPU por diferença entre leituras e documenta que a primeira chamada tende a ser inexata. O runtime faz prime e agenda `due_now`; falta um teste determinístico que prove que a primeira CPU publicada respeita o intervalo mínimo útil. [Documentação `System`](https://docs.rs/sysinfo/0.33.1/sysinfo/struct.System.html), [`MINIMUM_CPU_UPDATE_INTERVAL`](https://docs.rs/sysinfo/0.33.1/sysinfo/constant.MINIMUM_CPU_UPDATE_INTERVAL.html).
2. **Falha de RAM pode descartar CPU válida.** O sample atual agrega ambas num único `Option<MacSystemSample>`. Uma falha em counters Mach/page size/pressure pode fazer CPU e RAM falharem juntas mesmo que a leitura CPU esteja válida.
3. **O custo atual da janela aberta não está medido no HEAD.** Há soaks anteriores do indicador fechado/idle, mas não foi localizado relatório comparativo do HEAD com “Uso do sistema” realmente visível, embora o ADR 0004 o exija.
4. **CPU ainda não é uma seção da janela.** `SystemUsageSection` cobre apenas RAM e GPU; a seam `SystemUsageSampling` expõe `memory(cycle)`, `gpu()` e processos de memória. O cache interno já compartilha `MacSystemSample` entre consumidores da mesma `SamplingCycle`, mas essa propriedade não está expressa no contrato da sessão.
5. **O sampler de processos é recriado.** A cada request de 6 s, uma thread efêmera cria uma nova `sysinfo::System` e atualiza memória dos processos. Reter o sampler só durante a sessão visível pode reduzir alocação, mas precisa de comparação A/B porque também aumenta memória retida.
6. **CPU por processo é substancialmente mais cara.** Exige duas coletas/deltas, retenção de estado, normalização explícita em máquinas multicore e tratamento de processos que nascem/morrem.

## Proposta priorizada

### P0 — correção e baseline antes de nova UI

#### 1. Tornar o warm-up de CPU determinístico

Impedir a publicação da primeira leitura antes de `MINIMUM_CPU_UPDATE_INTERVAL`, sem atrasar a RAM e sem criar um wake extra.

Aceitação mínima:

- relógio falso cobre prime + tick imediato;
- CPU só fica disponível no primeiro tick elegível;
- intervalos 1/2/60 s e sleep/wake não criam burst/catch-up;
- o estado acessível distingue indisponível, antigo e disponível.

#### 2. Separar outcomes de CPU e RAM

Trocar o resultado monolítico por estados independentes, preservando a última leitura de cada métrica como stale quando apropriado.

Aceitação mínima:

- CPU ok/RAM falhou e RAM ok/CPU falhou;
- recuperação de stale para disponível;
- falha de pressão desconhecida não causa panic;
- nenhum novo OS read e nenhum redraw sem mudança semântica.

#### 3. Medir o HEAD fechado e aberto

Executar um soak comparável com preferências e hardware registrados:

- indicador fechado/idle;
- “Uso do sistema” aberta e visível;
- CPU média/picos, RSS/footprint, `IDLEW`, context switches, OS reads, redraws e duração do sample;
- mesmo intervalo e mesma duração nos dois cenários.

O resultado deve estabelecer orçamento antes de otimizar ou adicionar CPU à janela.

### P1 — aprofundar a arquitetura de coleta

#### 4. Explicitar no seam o `SystemSample` já compartilhado por ciclo

Aprofundar `SystemUsageSampling`: em vez de `memory(cycle)`, devolver um tipo de domínio contendo outcomes independentes de CPU e memória. O runtime **já reutiliza** o `MacSystemSample` cacheado quando o indicador e a sessão consultam a mesma `SamplingCycle`; portanto, esta mudança não deve ser vendida como eliminação de uma leitura duplicada existente. Ela torna o contrato explícito, oferece CPU à sessão sem vazar tipos macOS/sysinfo e dá nome às falhas parciais.

Alternativa mínima: adicionar à interface um acesso a CPU que receba a mesma `SamplingCycle` e reutilize o cache atual. O tipo unificado é preferível somente se a implementação demonstrar que reduz acoplamento e simplifica invariantes/testes; qualquer ganho de performance precisa de contadores ou benchmark, não de inferência arquitetural.

Benefícios:

- CPU detalhada pode ser adicionada preservando a reutilização já existente;
- falhas parciais ficam explícitas;
- testes tornam explícita a propriedade atual de exatamente um OS read por ciclo;
- política de stale/gap permanece dentro de `SystemUsageSession`.

#### 5. Avaliar sampler de processos retido somente enquanto visível

Fazer A/B antes de decidir. O candidato mantém `sysinfo::System` durante a sessão, continua atualizando só o subconjunto necessário e libera tudo ao fechar.

Gates: zero recurso quando fechada, cancelamento/single-flight, processos terminados, reordenação durante interação, footprint aberto e liberação após close.

### P2 — seção CPU mínima em “Uso do sistema”

Após P0/P1 e a decisão de ADR, adicionar uma terceira seção dentro da janela:

```text
┌──────────────────────────────────────────────────────────┐
│ Uso do sistema                                           │
│                                                          │
│  [ CPU ]  [ RAM ]  [ GPU ]                               │
│                                                          │
│  CPU                                      37%            │
│  Uso total · amostra compartilhada                       │
│                                                          │
│  histórico dos últimos 5 minutos                         │
│  ▁▂▅▃▇▄▂▁                                                │
│                                                          │
│                     [Abrir Monitor de Atividade]          │
└──────────────────────────────────────────────────────────┘
```

Recorte inicial:

- CPU global como valor principal;
- o mesmo histórico de 5 min/150 pontos, somente em memória;
- lacunas em sleep/falha, sem interpolação;
- resumo textual completo do gráfico para VoiceOver;
- atalho para abrir o Monitor de Atividade;
- nenhuma nova configuração de menu bar e nenhum timer quando a janela estiver fechada.

O atalho nativo é preferível a implementar finalização de processos: preserva escopo, evita controles destrutivos e entrega uma rota de investigação completa.

`User/System/Idle` fica fora deste primeiro recorte. O backend atual fornece uso global e por CPU via `sysinfo 0.33.1`, mas a API pública inspecionada não fornece essa decomposição. Antes de propô-la na UI, uma investigação técnica separada deve escolher uma fonte pública/estável, definir deltas e normalização 0–100, cobrir warm-up e sleep/wake, isolar falhas e medir custo. O ADR de produto deve aprovar apenas o recorte cuja fonte tenha sido validada.

### P3 — somente após evidência de uso e custo

| Opção | Valor | Por que esperar |
| --- | --- | --- |
| **Top apps por CPU** | Responde “quem está causando?” | Requer delta entre samples, sampler retido, normalização multicore e soak específico. |
| **Por core / clusters P e E** | Diagnóstico avançado em Apple Silicon | Alta cardinalidade, ruído visual, diferenças de hardware e maior custo de layout/AX. |
| **User/System/Idle** | Explica a origem da carga global | O backend atual não oferece essa decomposição; requer pesquisa de API/counters, semântica de delta e validação antes da decisão de UI. |
| **Alertas por CPU/RAM** | Aviso proativo de pressão persistente | Precisa definir limiar + duração + cooldown + sono + recuperação; pico isolado não deve notificar. |
| **Suavização de CPU** | Número visualmente mais estável | Pode mascarar picos e atrasar significado; se adotada, deve existir só na apresentação e preservar raw internamente. |
| **Histórico persistente** | Investigação retrospectiva longa | Contraria o ADR atual, amplia privacidade/storage e não é necessário para “pico ou tendência?”. |

## O que não trazer

- terceiro item ou terceira métrica permanente na menu bar;
- dezenas de widgets, gauges e combinações;
- dashboard personalizável ao estilo KDE/Usage;
- refresh subsegundo ou 1 s como novo default sem benchmark;
- sensores de temperatura/fans que exijam helper, privilégios ou APIs privadas;
- controle de processos/“Forçar Encerrar” dentro do Statlet;
- persistência de séries ou nomes/processos no Histórico de atividade;
- alertas baseados em uma única amostra;
- igualdade numérica prometida com “App Memory” do Activity Monitor sem oracle público da fórmula.

## Sequência de entrega sugerida

| Etapa | Entrega | Gate para avançar |
| --- | --- | --- |
| **0. Correção** | warm-up testável + outcomes CPU/RAM independentes | testes de startup, falhas parciais, sleep/wake e AX verdes |
| **1. Medição** | baseline fechado/aberto no HEAD | orçamento explícito de CPU/footprint/wakes/redraws |
| **2. Seam** | contrato explícito para o sample já compartilhado por ciclo | prova do OS read único existente; outcomes independentes; zero coleta da janela fechada |
| **3. Produto** | decisão/emenda do ADR + seção CPU mínima | revisão de produto e arquitetura aceita |
| **4. Validação** | testes, soak comparativo e QA manual | AppKit, Light/Dark, Retina/notch, VoiceOver, Full Keyboard Access, sleep/wake |
| **5. Expansão opcional** | top CPU apps ou per-core | uso real + custo medido justificam a complexidade |

## Critérios de aceitação da seção CPU

- mantém um único item e duas linhas no indicador compacto;
- janela fechada não adiciona deadline, thread, processo, leitura ou persistência;
- janela aberta compartilha a observação do ciclo em vez de duplicar a coleta;
- história permanece limitada a 150 pontos/5 minutos e cria lacunas após pausas;
- CPU, RAM e GPU falham independentemente;
- texto e acessibilidade não dependem apenas de cor ou do desenho do gráfico;
- navegação por teclado e VoiceOver alcança tabs, valores, resumo do histórico e atalho;
- layout passa em tamanhos de texto, Light/Dark, Increase Contrast, Retina e displays com notch;
- soak fechado mantém os gates v1 e o aberto respeita o orçamento definido na etapa 1.

## Limites desta pesquisa

- Sinais de adoção são snapshots por canal e podem mudar; não constituem ranking global.
- Não houve instalação, comparação visual ou medição local dos concorrentes.
- Afirmações de eficiência dos fabricantes não foram tratadas como benchmarks independentes.
- As páginas públicas de iStat e Usage não informam todas as opções/defaults de refresh.
- Não foi encontrada documentação específica de leitor de tela para todos os monitores; acessibilidade real dos gráficos exige teste manual.
- A fórmula pública exata de “App Memory” do Activity Monitor não está documentada; paridade deve ser avaliada com tolerância, versão do macOS e hardware registrados.

## Fontes primárias principais

- Bjango: [iStat Menus](https://bjango.com/mac/istatmenus/), [ajuda v7](https://bjango.com/help/istatmenus7/welcome/), [histórico de versões](https://bjango.com/mac/istatmenus/versionhistory/).
- Stats: [repositório oficial](https://github.com/exelban/stats) e [README fixado](https://github.com/exelban/stats/blob/327eb11160e529cd4ca4e1c82007154941550c2e/README.md).
- MenuMeters: [repositório oficial](https://github.com/yujitach/MenuMeters) e [README fixado](https://github.com/yujitach/MenuMeters/blob/e91b746debd15777012968a4d247a074d10402f6/README.md).
- Usage: [site](https://usage.pro/), [App Store](https://apps.apple.com/us/app/usage-device-monitor/id1561788435?platform=mac), [Helper](https://usage.pro/mac/helper).
- Apple: [CPU no Activity Monitor](https://support.apple.com/guide/activity-monitor/actmntr43452/mac), [memória](https://support.apple.com/guide/activity-monitor/view-memory-usage-actmntr1004/mac), [frequência](https://support.apple.com/guide/activity-monitor/actmntr2224/mac).
- Microsoft: [Task Manager](https://learn.microsoft.com/en-us/troubleshoot/windows-server/support-tools/support-tools-task-manager), [memória](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information).
- GNOME: [System Monitor](https://help.gnome.org/gnome-system-monitor/).
- KDE: [Plasma System Monitor](https://apps.kde.org/plasma-systemmonitor/).
- btop: [repositório oficial](https://github.com/aristocratos/btop).
- sysinfo 0.33.1: [crate](https://docs.rs/sysinfo/0.33.1/sysinfo/), [`System`](https://docs.rs/sysinfo/0.33.1/sysinfo/struct.System.html), [`MINIMUM_CPU_UPDATE_INTERVAL`](https://docs.rs/sysinfo/0.33.1/sysinfo/constant.MINIMUM_CPU_UPDATE_INTERVAL.html).
