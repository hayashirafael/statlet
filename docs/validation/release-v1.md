# Checklist final da v1

Registro dos gates automáticos e das verificações manuais do bundle de produção. Um item não marcado continua sendo um gate humano explícito; ele não deve ser convertido em alegação de validação.

## Build e distribuição

- [x] Build release `arm64` com LTO, um codegen unit, `panic=abort` e símbolos removidos.
- [x] Bundle declara `LSUIElement=true`, macOS 14 mínimo, versão 1.0.0 e categoria Utilities.
- [x] Privacy manifest declara disk space para exibição (`85F4.1`), nenhuma coleta e nenhum tracking.
- [x] Bundle contém AppIcon, LICENSE Apache 2.0, NOTICE com linhagem do featherbar e licenças transitivas geradas do lockfile.
- [x] Verificação de assinatura, arquitetura, plists, recursos, ZIP extraído e SHA-256 passa localmente.
- [x] CI executa format, testes, Clippy sem warnings e contrato arm64 em runner macOS 15, verificando deployment target macOS 14 no Mach-O.
- [ ] Developer ID, notarização, staple e avaliação do Gatekeeper — bloqueado pelas credenciais externas documentadas em `docs/release/signing-and-notarization.md`.

## Performance

- [x] Soak de 30 minutos do bundle de produção concluído com relatório e 169 amostras versionados.
- [x] RSS sem crescimento não limitado: 55.088 → 31.824 KiB, crescimento final de −23.264 KiB e pico de 0 KiB acima da primeira amostra pós-warm-up.
- [x] CPU amostrada média de 0,122485% no cenário idle, abaixo do guard de 1%.
- [x] Idle wakeups registrados pelo contador `IDLEW` do macOS: delta 0; context switches mantidos apenas como contexto secundário.

## UI, acessibilidade e lifecycle

- [ ] Launch e percurso básico em hardware Apple Silicon com macOS 14 — deployment target verificado no Mach-O, mas não há máquina Sonoma disponível nesta execução.
- [x] Indicador e janela de Preferências inspecionados em Retina, Light e Dark.
- [x] Launch via Launch Services mostra uma janela útil sem depender do item da menu bar.
- [x] Badge usa `!`/`×` além de cor e o status item possui descrição completa.
- [x] Core automatiza debounce, episódio ativo e schedule após sleep sem alerta falso ou rajada.
- [ ] Percurso completo com Full Keyboard Access.
- [ ] Percurso completo com VoiceOver e Accessibility Inspector.
- [ ] Increase Contrast em Light e Dark para indicador e três janelas.
- [ ] Notificação real: autorização, corpo, clique e ausência de repetição no mesmo episódio.
- [ ] Sleep/wake físico durante debounce e episódio ativo.
- [ ] Troca física entre dois monitores e desconexão com janelas abertas.
- [ ] Finder e Spotlight após fechar a janela, confirmando reutilização e foco.

Detalhes do checklist específico estão em `docs/validation/accessibility-lifecycle.md`.

## Segurança funcional

- [x] Integração desativada por padrão e badge de disco ausente nesse estado.
- [x] O detector executa somente `mo --version` com timeout, saída limitada e grupo de processo encerrado.
- [x] A única ação do Mole abre `mo clean` interativo no Terminal após clique explícito.
- [x] Nenhuma limpeza, resultado ou espaço liberado é inventado no histórico.
- [x] Não há `sudo`, limpeza automática, parsing de prompt/ANSI nem instalação automática do Mole.
