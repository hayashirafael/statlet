# Personalização curada do indicador

Status: aprovado para planejamento em 12 de agosto de 2026.

## Objetivo

Permitir que cada pessoa adapte a aparência e a frequência do indicador compacto sem transformar o Statlet em um sistema de widgets. A evolução deve preservar as duas linhas simultâneas de CPU e RAM, a leitura acessível, a largura estável e o modelo de baixo consumo aprovado para a v1.

Esta especificação substitui, para a próxima versão, os itens de `docs/product/v1.md` que deixavam intervalos configuráveis e presets alternativos fora do escopo. Ela não altera o cálculo das métricas, o monitoramento de disco nem o contrato de segurança do Mole.

## Referências e direção adotada

Foram comparadas três organizações:

1. duas áreas, `Indicador` e `Disco e Mole`, com prévia fixa;
2. uma única página rolável;
3. uma sidebar dividida por assunto.

A primeira foi escolhida porque dá contexto imediato à personalização e preserva a experiência existente de disco sem fragmentar uma configuração ainda pequena. A direção combina a prévia contextual e as configurações por item do [iStat Menus](https://diagnostics.bjango.com/help/istatmenus7/welcome/) com controles nativos semelhantes aos usados pelo [Stats](https://github.com/exelban/stats), mas mantém um conjunto de opções menor e coerente com o indicador compacto.

## Princípios

1. O indicador compacto continua sendo o produto principal.
2. CPU e RAM continuam simultâneas, nessa ordem e em duas linhas.
3. A configuração oferece liberdade dentro de limites que protegem legibilidade, desempenho e recuperação.
4. Toda alteração válida produz feedback imediato.
5. Cor nunca é a única forma de comunicar significado.
6. A personalização não adiciona polling, worker permanente nem coleta de métricas.
7. Os padrões reproduzem exatamente a apresentação e a frequência da v1.

## Experiência da janela

A janela **Preferências do Statlet** terá um seletor nativo no topo com duas áreas:

- **Indicador**;
- **Disco e Mole**.

`Disco e Mole` preserva os controles e o comportamento atuais. A área `Indicador` terá uma prévia fixa no topo e quatro grupos abaixo dela: **CPU e RAM**, **Rótulos**, **Tipografia** e **Atualização**.

A prévia mostra a mesma apresentação nas aparências clara e escura, lado a lado e na escala usada pela menu bar. As duas usam o snapshot mais recente já coletado pelo runtime; não existe timer exclusivo para a prévia. A prévia é uma aproximação, pois wallpaper, transparência, contraste aumentado e o estado pressionado do status item podem mudar o fundo real da menu bar.

Alterações válidas atualizam, nesta ordem lógica, o estado em memória, o indicador real, as duas prévias e o arquivo de preferências. O efeito visual é imediato e o salvamento é automático; não há botão **Aplicar**.

## CPU e RAM

CPU e RAM têm controles independentes de cor. Cada métrica oferece dois modos:

- **Dinâmica**: preserva o comportamento atual. CPU usa verde entre 0–39%, laranja entre 40–69% e vermelho entre 70–100%. RAM acompanha o memory pressure do macOS.
- **Fixa**: a cor não muda com carga ou severidade.

Uma cor fixa começa compartilhada entre as aparências clara e escura. O controle **Personalizar claro e escuro** revela duas variantes opcionais. Ao ativá-lo pela primeira vez, ambas são inicializadas com a cor compartilhada. Desativá-lo volta a usar a cor compartilhada sem apagar as variantes; reativá-lo recupera os últimos valores. Restaurar o grupo remove as variantes e volta ambas as métricas para o modo dinâmico.

### Controle de cor

Cada cor fixa usa um único controle lógico formado por:

- uma mini caixa `NSColorWell`, que abre o seletor nativo do macOS;
- um campo hexadecimal editável ao lado.

O comportamento é bidirecional:

- escolher uma cor atualiza o hexadecimal e as prévias;
- digitar seis dígitos válidos atualiza a caixa e as prévias;
- o campo aceita com ou sem `#` e normaliza para `#RRGGBB` maiúsculo;
- alpha não é aceito nem persistido;
- cores são convertidas e persistidas em sRGB.

Enquanto o campo estiver incompleto ou inválido, a última cor válida continua ativa. Ao completar seis dígitos, pressionar Return ou retirar o foco, o campo é validado. Um valor inválido recebe mensagem curta inline e acessível, sem modal e sem corromper a preferência válida.

## Rótulos

Um único controle **Mostrar rótulos C/R** governa as duas linhas. Não é possível ocultar apenas um deles.

Cada linha oferece seu próprio campo de rótulo. O campo aceita de 1 a 10 caracteres Unicode depois de remover espaços externos; valores vazios ou maiores são rejeitados sem alterar a preferência já válida. Um terceiro controle escolhe dez níveis decimais entre 0 e 1 espaço: `0`, `0,1` … `0,9` e `1 espaço`. O nível `10` preserva exatamente a largura visual do espaço literal legado; o rótulo não armazena padding e o renderer mede/aplica o delta explicitamente. O padrão compacto continua `C 18% / R 63%`.

Quando visíveis, existem três modos de cor:

- **Neutra**: usa a cor de texto semântica do macOS, como na v1;
- **Igual ao valor**: `C` acompanha a apresentação da CPU e `R` acompanha a apresentação da RAM, inclusive seus modos dinâmicos, fixos e variantes de aparência;
- **Personalizada**: usa seu próprio controle de cor fixa compartilhada, com a opção de variantes clara e escura descrita acima.

Quando ocultos, os rótulos e seus espaços são removidos. A apresentação passa, por exemplo, de `C 18% / R 63%` para `18% / 63%`. A largura permanece estável entre valores de `0%` a `100%` dentro da configuração escolhida.

Ocultar rótulos nunca reduz a descrição acessível: VoiceOver continua recebendo os nomes completos das métricas.

## Tipografia

A tipografia é global para CPU, RAM e rótulos. Ela contém:

- **Família**: qualquer fonte instalada no Mac;
- **Tamanho**: inteiro de 9 a 14 pt, padrão 12 pt;
- **Peso**: `Regular`, `Médio` ou `Negrito`, padrão `Médio`.

A fonte padrão é a monoespaçada do sistema. Um botão com o nome da fonte abre um seletor com busca, lista de famílias instaladas e amostra `C 42% / R 68%` renderizada em cada família. Fontes proporcionais são permitidas. O peso é uma intenção semântica; quando a família não contém a face exata, AppKit usa a correspondência disponível mais próxima.

A preferência persiste o nome da família, pois peso é uma escolha separada. Se ela deixar de existir, o renderer usa a monoespaçada do sistema sem sobrescrever a escolha salva. A janela informa o fallback e volta automaticamente à família escolhida se ela reaparecer.

### Dimensões e largura estável

Para cada configuração, o renderer mede todos os valores inteiros de `0%` a `100%` e reserva a maior largura de cada linha. Mudanças normais de métricas não deslocam outros itens da menu bar. Mostrar ou ocultar rótulos, trocar a fonte ou alterar seu tamanho pode recalcular a largura uma vez, pois são ações explícitas da pessoa.

O badge opcional de disco continua podendo acrescentar sua largura apenas quando existir, como na v1.

O seletor de fontes mostra sua amostra antes da escolha. A prévia exibe um aviso não bloqueante quando:

- as duas linhas, incluindo o espaçamento aprovado, excederem os 22 pt disponíveis; ou
- a largura calculada exceder duas vezes a largura da configuração padrão com rótulos visíveis.

A escolha continua permitida. O aviso explica o risco de corte ou ocupação excessiva e oferece **Restaurar tipografia**, sem mudar silenciosamente a fonte.

## Atualização

CPU e RAM compartilham um único intervalo inteiro de 1 a 60 segundos. O padrão é 2 segundos.

A interface usa um campo numérico com stepper e o sufixo `segundos`. Valores fora da faixa não são aplicados. O texto de ajuda explica que números menores produzem atualização mais frequente e maior consumo de recursos.

Uma alteração válida reprograma imediatamente o próximo ciclo a partir do momento da mudança. Ela não dispara uma coleta extra, não cria outro timer e não altera a amostragem de disco, que continua independente.

## Restaurar e desfazer

Cada um dos quatro grupos oferece **Restaurar** e afeta apenas suas próprias preferências.

O rodapé da área `Indicador` oferece **Restaurar indicador aos padrões…**. A confirmação resume que CPU/RAM, rótulos, tipografia e intervalo serão restaurados. Ela nunca altera `Disco e Mole`.

Após a restauração global, a janela mostra uma ação **Desfazer** enquanto permanecer aberta. É um undo de um nível: restaura o snapshot completo do indicador imediatamente anterior à última restauração global. Fechar a janela ou realizar outra restauração global descarta o snapshot anterior. O atalho Command-Z aciona a mesma operação enquanto ela estiver disponível.

Se a pessoa fizer outras alterações após a restauração, **Desfazer restauração** continua disponível e substitui essas alterações pelo snapshot anterior. O texto da ação explicita esse efeito antes da execução.

Os padrões são:

- CPU e RAM em modo dinâmico;
- rótulos visíveis e neutros;
- fonte monoespaçada do sistema, 12 pt, peso médio;
- intervalo de 2 segundos.

## Modelo e limites de responsabilidade

As responsabilidades serão separadas em unidades independentes:

- **Preferências do indicador**: tipos validados de cor, rótulos, tipografia e intervalo;
- **Apresentação**: transforma snapshot, preferências e aparência do sistema em linhas e segmentos;
- **Renderer**: mede e desenha o indicador real e as prévias usando a mesma apresentação;
- **Agendamento**: determina a próxima amostragem a partir do intervalo válido;
- **Persistência**: migra, carrega e salva a versão armazenada;
- **Janela**: traduz controles AppKit em eventos do domínio e apresenta erros, sem calcular métricas ou regras de apresentação.

O fluxo de uma alteração é:

```text
controle alterado
→ valor validado
→ preferência atualizada em memória
→ indicador e prévias redesenhados
→ preferência completa salva atomicamente
```

Alterar cor, rótulo ou tipografia apenas redesenha. Alterar o intervalo também reprograma o próximo wakeup. Nenhuma dessas ações coleta CPU/RAM novamente.

## Persistência e migração

O schema de preferências passa da versão 1 para a versão 2. A versão 2 adiciona um bloco de preferências do indicador e mantém `moleIntegrationEnabled` e `warningThreshold` sem mudança semântica. A versão 3 representa o espaçamento de rótulo em décimos (`0..10`): ao carregar v2, `0` permanece `0` e qualquer valor legado de `1..4` migra para `10`; v1 ou arquivo ausente usam o padrão `10`.

Ao carregar a versão 1, o Statlet preserva os valores existentes de Disco e Mole e preenche o indicador com os padrões listados acima. O próximo salvamento grava a versão 3. Arquivo ausente, corrompido ou com versão não suportada continua produzindo defaults seguros.

Cada alteração válida salva o documento completo por substituição atômica, como hoje. Se a gravação falhar:

- a configuração continua ativa apenas na sessão;
- a janela mostra uma mensagem discreta de que não foi possível salvar, com a ação **Tentar novamente**;
- **Tentar novamente** persiste o estado completo atualmente ativo, não o valor antigo que falhou;
- a próxima alteração válida tenta persistir novamente o estado completo;
- o app nunca exibe sucesso de salvamento sem confirmação do filesystem.

## Aparência, contraste e acessibilidade

Uma cor fixa significa fixa em relação à carga, não necessariamente idêntica entre aparências. A pessoa pode manter um único RGB ou ativar variantes clara e escura.

As prévias calculam contraste contra fundos representativos claro e escuro. Texto pequeno abaixo de 4,5:1 recebe aviso não bloqueante. O aviso nunca corrige ou troca automaticamente uma cor. Ele também explica que a prévia não consegue garantir contraste contra todo wallpaper ou estado da menu bar.

O status item permanece um único elemento acessível. Sua descrição inclui, independentemente de rótulos ou cores, CPU, RAM, memory pressure e badge de disco aplicável. Exemplo: `CPU 42%, RAM 68%, pressão de memória normal`.

Os controles usam AppKit nativo e oferecem:

- ordem de Tab previsível: modo, caixa de cor, hexadecimal e próximo grupo;
- labels curtas e visíveis, não apenas placeholders;
- valor hexadecimal anunciado pelo VoiceOver;
- Space/Return para abrir a caixa de cor;
- Return para validar o hexadecimal;
- erro inline exposto à árvore de acessibilidade;
- reação às mudanças de Light/Dark, Increase Contrast, Differentiate Without Color e Reduce Transparency.

## Desempenho

A mudança é compatível com o ADR 0001 e deve preservar:

- event loop principal com `ControlFlow::WaitUntil`;
- nenhum worker permanente novo;
- renderer e atributos reutilizados por configuração;
- `autoreleasepool` por ciclo;
- nenhuma lista permanente de processos;
- nenhuma amostragem exclusiva para a prévia.

Trocas de aparência ou preferência causam um redesenho pontual. O soak de release deve comparar CPU média, RSS, physical footprint e wakeups com a evidência da v1. Um intervalo menor escolhido pela pessoa pode aumentar wakeups; a validação de regressão usa o padrão de 2 segundos.

## Tratamento de erros

- **Hex inválido**: mantém a última cor válida e mostra erro inline.
- **Fonte ausente**: usa fallback sem perder a preferência e informa a situação.
- **Falha de persistência**: mantém estado da sessão, informa e tenta novamente na próxima mudança.
- **Valor de intervalo inválido**: não aplica nem reprograma; mostra a faixa aceita.
- **Configuração visual impraticável**: mostra aviso e restauração contextual, mas respeita a escolha.
- **Preferências antigas**: migra v1 para v2 preservando Disco e Mole.
- **Arquivo corrompido ou versão desconhecida**: usa defaults seguros.

## Testes automatizados

Os testes devem cobrir:

1. defaults e validação de tamanho, peso, intervalo e hexadecimal;
2. conversão e round-trip sRGB `#RRGGBB` sem alpha;
3. migração v1 → v2 preservando Disco e Mole;
4. round-trip v2 e substituição atômica;
5. falha de persistência e nova tentativa posterior;
6. CPU/RAM dinâmica versus fixa;
7. cor compartilhada e variantes clara/escura;
8. rótulos neutros, iguais ao valor, personalizados e ocultos;
9. medição estável para `0%` a `100%` com fontes proporcionais e monoespaçadas;
10. fonte ausente, fallback e recuperação quando reinstalada;
11. reprogramação do intervalo sem timer adicional ou coleta imediata;
12. restauração por grupo, restauração global e undo de um nível;
13. descrição acessível independente da apresentação visual;
14. ausência de mudanças nos cálculos de CPU, RAM, memory pressure e disco.

## Validação manual

Antes de release, validar:

- seletor nativo, hexadecimal e sincronização bidirecional;
- aplicação imediata no indicador e nas duas prévias;
- fontes proporcionais, monoespaçadas, muito largas e ausentes;
- Light Mode, Dark Mode, Increase Contrast e Reduce Transparency;
- Full Keyboard Access, VoiceOver e Accessibility Inspector;
- menu bar com notch, escalas Retina e troca de monitor;
- sleep/wake e mudança de aparência enquanto o app está aberto;
- mensagens e recuperação após falha real de escrita;
- soak com intervalo padrão de 2 segundos e comparação com a v1.

## Fora do escopo

- reordenar CPU e RAM;
- ocultar somente uma das métricas ou somente um dos rótulos;
- intervalos independentes por métrica;
- tamanhos, famílias ou pesos de fonte independentes por métrica;
- transparência de cor;
- personalizar os limiares ou as cores do modo dinâmico;
- presets, importação ou exportação de temas;
- gráficos, processos ou novas métricas permanentes;
- mudanças em Disco e Mole além da nova organização da janela.

## Critérios de sucesso

O design está cumprido quando uma pessoa consegue personalizar cores, rótulos, fonte e intervalo sem perder a leitura simultânea de CPU/RAM; recuperar qualquer grupo ou todos os padrões do indicador; compreender problemas de contraste e dimensões antes de aceitá-los; e reiniciar o app com suas escolhas preservadas. Tudo isso deve ocorrer sem regressão nas métricas, na acessibilidade ou no modelo de performance da v1.
