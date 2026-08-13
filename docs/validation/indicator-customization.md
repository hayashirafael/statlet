# Validação da personalização do indicador

Status: gates automatizados, package contract e soak de performance aprovados em 12 de agosto de 2026; validação visual e assistiva não executada sem autorização de foreground.

Este checklist valida a personalização aprovada para a próxima versão sem reclassificar a história da v1.0.0. O contrato de produto continua sendo CPU acima de RAM, simultâneas em um único item, com cálculos de métricas e agenda de disco inalterados.

## Limite desta rodada

- O Statlet não foi aberto nem ativado para inspeção visual.
- Nenhuma janela, item da menu bar, seletor de cor ou seletor de fonte foi operado.
- O soak autorizado inicia o executável do bundle diretamente em um terminal de background, usando um arquivo v2 isolado pelo caminho específico `STATLET_PREFERENCES_PATH`, sem alterar `$HOME`, Launch Services ou interagir com a UI. Ele mede o processo; não valida aparência ou acessibilidade.
- Itens dependentes de visão, foco, tecnologia assistiva, display ou ciclo físico permanecem desmarcados e não são alegações de execução.

## Evidência automatizada

Os comandos abaixo foram executados no mesmo tree preparado para o commit de validação:

- [x] `rtk cargo fmt --all -- --check`: exit 0.
- [x] `rtk bash -n scripts/*.sh tests/package_contract.sh`: exit 0.
- [x] `rtk cargo test --all-targets --all-features --locked`: 185 testes em 22 suites, 0 falhas.
- [x] `rtk cargo clippy --all-targets --all-features --locked -- -D warnings`: 0 issues.
- [x] `rtk git diff --check`: exit 0.
- [x] `rtk bash tests/package_contract.sh`: bundle, arquitetura arm64, `Info.plist`, privacy manifest, licenças, assinatura, ZIP extraído e checksum aprovados.

Cobertura automatizada relevante:

- [x] hexadecimal aceita `#RRGGBB` ou `RRGGBB`, normaliza para maiúsculas e rejeita alpha, formato curto, não ASCII e dígitos inválidos;
- [x] CPU/RAM aceitam modo dinâmico ou fixo, cor compartilhada e variantes Light/Dark; as cores escolhidas não são substituídas por avisos de contraste;
- [x] rótulos podem ser neutros, acompanhar o valor, usar cor fixa ou ser ocultados em conjunto; a descrição acessível continua nomeando CPU e RAM;
- [x] famílias de fonte instaladas podem ser filtradas e selecionadas; tamanhos aceitam somente inteiros de 9 a 14 pt e pesos regular, médio e negrito;
- [x] fonte ausente usa fallback sem sobrescrever a família pedida, invalida o cache após notificação de fontes e pode se recuperar quando reinstalada;
- [x] layout mede `0%` a `100%` e mantém largura estável para fontes monoespaçadas e proporcionais; diagnósticos de largura são não bloqueantes;
- [x] o intervalo compartilhado aceita somente inteiros de 1 a 60 segundos, incluindo os limites 1, 2 e 60; o padrão é 2;
- [x] mudar o intervalo reprograma métricas sem coleta imediata, timer adicional ou atraso na agenda independente de disco;
- [x] restaurações por grupo, restauração global, descarte ao fechar e undo global de um nível preservam Disco/Mole;
- [x] falha ao salvar permanece visível, retry salva o documento completo mais recente e sucesso limpa o aviso;
- [x] as prévias Light/Dark compartilham composer, resolução de fonte, layout e renderer com o indicador real, sem timer ou polling próprio;
- [x] contraste é calculado contra fundos representativos claro/escuro com aviso abaixo de 4,5:1; wallpaper, transparência e estado real da menu bar permanecem limitações declaradas;
- [x] Increase Contrast invalida cores semânticas; Differentiate Without Color preserva valores e símbolos; Reduce Transparency reaplica fundos opacos das prévias.

## Package contract e preferências

- [x] O fixture v1 original continua provando que Mole ativo bloqueia um soak rotulado como baseline.
- [x] O fixture v2 dedicado prova que `refreshInterval` diferente de 2 bloqueia o baseline padrão, sem substituir os testes unitários da persistência.
- [x] O bundle exato do HEAD de código `a2dcb90` usado no soak final v2 foi verificado: executável SHA-256 `c4f91b5a57c659248b71fcb0fa89dc50f21ecdc8b43ab67279cd596d4dc9fbe5`; ZIP SHA-256 `15897add92d44cdb6da26a99f43d928ab2c154e869f5bd8fa7603943a337824a`.

## Ambiente observado sem foreground

Leituras de sistema, sem abrir ou ativar o Statlet, identificaram Mac16,1 com Apple M4, arm64, macOS 26.5.2 (25F84), display interno Liquid Retina XDR 3024 × 1964 Retina e display externo HP V206hz 1600 × 900 a 60 Hz. Essa enumeração registra o host do soak; não transforma notch, Retina ou troca de display em gates manuais executados.

## Gates manuais não executados

Todos os itens abaixo estão desmarcados por falta de autorização de foreground nesta rodada:

- [ ] Cor nativa e hexadecimal: sincronização bidirecional, entrada incompleta/inválida, paste, Return, blur e rejeição de alpha.
- [ ] Light Mode e Dark Mode: indicador real e duas prévias com cores dinâmicas, fixas, compartilhadas e variantes.
- [ ] Limitações de contraste: avisos legíveis sem corrigir a cor e comparação com wallpaper, transparência e estado pressionado reais.
- [ ] Qualquer fonte instalada: busca, amostra, seleção e aplicação de famílias monoespaçadas, proporcionais e muito largas.
- [ ] Fonte ausente e reinstalada: fallback visível sem perder a escolha e recuperação após notificação do sistema.
- [ ] Tamanhos 9, 10, 11, 12, 13 e 14 pt; rejeição visual de 8 e 15.
- [ ] Pesos regular, médio e negrito, incluindo fallback para a face mais próxima.
- [ ] Intervalos 1, 2 e 60 segundos; limites do stepper, validação do campo e efeito observado na atualização.
- [ ] Restaurar CPU/RAM, rótulos, tipografia e intervalo separadamente.
- [ ] Restaurar todo o indicador com confirmação, preservando Disco/Mole, e desfazer uma vez pelo botão e por Command-Z.
- [ ] Undo/redo comum de campo quando não houver restauração global disponível.
- [ ] Falha real de escrita, aviso persistente, retry com o documento mais recente e limpeza do aviso após sucesso.
- [ ] VoiceOver e Accessibility Inspector: papéis, nomes, valores hexadecimais, ajuda, estados, ordem, prévias textuais e rótulos `C`/`R` ocultos.
- [ ] Full Keyboard Access: Tab/Shift-Tab, foco visível, Space/Return, color wells, font picker, resets, undo e retry condicionais.
- [ ] Increase Contrast em Light e Dark.
- [ ] Differentiate Without Color com valores CPU/RAM e símbolos `!`/`×` ainda presentes.
- [ ] Reduce Transparency com fundos opacos das prévias e legibilidade da janela.
- [ ] MacBook com notch: largura e posição do indicador em 0%, 9%, 10%, 99% e 100%.
- [ ] Escalas Retina e mudança de resolução.
- [ ] Troca e desconexão de display, preservando uma única janela e estado.
- [ ] Sleep/wake com intervalo padrão e alterado, sem burst, alerta falso ou duplicado.
- [ ] Comparação visual entre indicador real e prévias após mudança de aparência enquanto o app está aberto.

## Soak do padrão de 2 segundos

- [x] Um arquivo v2 isolado confirmou explicitamente `moleIntegrationEnabled: false` e `indicator.refreshInterval: 2`; o caminho específico do Statlet evitou qualquer alteração das preferências reais.
- [x] O executável do bundle foi iniciado diretamente em background, sem `open`, ativação ou interação.
- [x] O re-soak v2 solicitado depois das otimizações e da correção final durou 1.800 segundos, observou 1.807 segundos, coletou 163 amostras a cada 10 segundos e excluiu 10 segundos de warm-up.
- [x] CPU média, crescimento e pico de RSS, physical footprint, idle wakeups e context switches foram registrados e comparados à v1.

Baseline v1.0.0 para comparação: 1.810 segundos observados; CPU média 0,122485%; RSS de 55.088 para 31.824 KiB; pico de RSS 0 KiB acima da primeira amostra; physical footprint de 20.595.528 para 19.989.320 bytes, pico final registrado de 20.841.288 bytes; `IDLEW` 0; 20.856 context switches, ou 11,52/s. A evidência fonte permanece em [`soak-v1`](soak-v1/).

Resultado final v2 da personalização no padrão de 2 segundos: CPU média 0,325767%; RSS de 118.480 para 38.384 KiB, crescimento final -80.096 KiB e pico 0 KiB acima da primeira amostra; physical footprint de 50.496.472 para 48.595.928 bytes, com pico histórico de processo de 109.659.072 bytes no snapshot final; `IDLEW` 0; 21.641 context switches, ou 11,98/s. Os gates de CPU (< 1%), crescimento final de RSS (< 10 MiB) e pico de RSS (< 20 MiB) passaram. A evidência fonte está em [`soak-indicator-final-v2`](soak-indicator-final-v2/).

Comparação com a v1: CPU aumentou 0,203282 ponto percentual e aproximadamente 2,66 vezes; o RSS final ficou 6.560 KiB acima da v1, sem crescimento; o physical footprint final aumentou 28.606.608 bytes. Idle wakeups permaneceram em zero. Context switches aumentaram 0,46/s, aproximadamente 1,04 vez; são contexto secundário, não um gate existente, e ficaram próximos à v1 nesta rodada.

Comparação com o soak anterior à otimização: CPU média caiu aproximadamente 35,7%, context switches/s caíram aproximadamente 56,7% e o physical footprint final caiu 1.671.192 bytes; o pico histórico de physical footprint, porém, aumentou 11.550.672 bytes. O alto footprint inicial/final inclui a janela de Preferências criada pelo contrato atual de launch direto; como não houve autorização de foreground, nenhuma janela foi inspecionada ou fechada para criar um cenário alternativo. A otimização melhorou o consumo contínuo, mas não autoriza alegar paridade quantitativa com a v1.

## Resultado residual

Automação e soak podem comprovar contratos de estado, pacote e consumo, mas não substituem os gates nativos acima. A release da personalização só pode alegar validação visual, VoiceOver, Full Keyboard Access, configurações de acessibilidade, notch, Retina, display change ou sleep/wake depois de uma rodada manual autorizada no hardware correspondente.
