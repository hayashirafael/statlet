# Implementação — identificador por texto, ícone do macOS ou PNG

Data: 2026-08-13  
Dispatch: `task_81542730f57b` / `ctx_2b5bf29fe2e6`  
Base confirmada: `a8f5ab43cd9f44312b115c2461a9628d5c967b70`

## Resultado

- CPU e RAM possuem modos independentes e mutuamente exclusivos: texto, SF Symbol nativo e PNG.
- Os padrões continuam sendo `C` e `R`; preferências v2 antigas sem o novo campo migram para esses padrões.
- O seletor nativo oferece catálogo curado de SF Symbols, importação por `NSOpenPanel`, miniatura, nome, erro em PT-BR e remoção por métrica.
- PNGs são validados antes da decodificação, limitados por bytes/pixels/alocação, orientados quando aplicável, reduzidos para até 24 px sem upscale, reencodados preservando alpha e gravados atomicamente como `cpu.png`/`ram.png` em Application Support.
- O renderer usa SF Symbols nativos ou o PNG normalizado, conserva a posição dos percentuais e cai para `C`/`R` em arquivo ausente, inválido ou imagem sem dimensão.
- A identidade do cache inclui modo, símbolo, métrica, metadados/fingerprint do PNG, fallback, tipografia e aparência; o cache LRU de imagens é limitado a 12 entradas.
- Controles possuem identificadores e rótulos acessíveis específicos para CPU/RAM, ordem explícita de teclado e resumo acessível da prévia.
- O inventário de licenças do bundle foi regenerado para as novas dependências de imagem.

## Ciclos TDD relevantes

- API de domínio/persistência começou com tipos ausentes; ficou verde após modos, símbolos curados, metadados e migração v2.
- Pipeline PNG começou sem módulo; ficou verde com normalização, limites defensivos, escrita atômica, nomes estáveis, remoção isolada e fingerprint.
- Fluxo reducer/effects começou sem eventos; ficou verde preservando preferências/arquivo anterior em falhas e salvando somente após sucesso.
- Renderer começou invalidando incorretamente o cache; ficou verde após incluir identidade visual, conteúdo e cor de fallback.
- Layout/AppKit começou sem slots/controles; ficou verde com matriz de não sobreposição, contratos de acessibilidade e navegação.
- A suíte completa revelou que a matriz legada de layout omitia os novos slots; a reprodução `mask 000: unexpected gap 240` ficou verde após representar a ordem real dos slots.
- Casos adicionais garantem prefixo gráfico compacto mesmo com rótulo textual longo e rejeição de nome PNG com caracteres de controle.

## Evidências executadas

- `rtk cargo test --test indicator_icon_customization --test png_icon_assets --test indicator_png_flow --test preferences_store --test indicator_preferences_flow --test indicator_presentation --test preferences_view --bin statlet` — 132 testes aprovados em 8 suítes.
- `rtk cargo test` — 235 testes aprovados em 27 suítes.
- `rtk cargo clippy --all-targets -- -D warnings` — sem achados.
- `rtk cargo fmt --check` — aprovado.
- `rtk git diff --check` — aprovado.
- `rtk bash -n scripts/*.sh tests/package_contract.sh` — aprovado.
- `rtk bash tests/package_contract.sh` — build release arm64/macOS 14+, assinatura ad hoc hardened runtime, bundle, ZIP, checksum, privacy manifest e avisos aprovados.
- Teste AppKit focal confirmou que os seis nomes curados produzem SF Symbols no runtime macOS atual.

## Limites de validação

- Nenhum app foi aberto, ativado ou operado em foreground por este worker; produção e dados de produção permaneceram intocados.
- Aparência Light/Dark real, VoiceOver, Full Keyboard Access e inspeção visual do painel/menu bar permanecem para a Task filha de QA visual do run.
- O teste dos SF Symbols comprova o runtime macOS atual e o bundle declara macOS 14+; compatibilidade visual no hardware/runtime mínimo ainda faz parte desse gate manual/CI específico.
- Não houve push nem PR.
