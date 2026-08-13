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

## Rework da revisão e do QA visual — Task `task_4c97d1a0eb05`

O checkpoint `670f1d3` recebeu quatro achados técnicos e nota visual 8,5/10. Este rework cobre os sete pontos antes de um novo QA:

- importação e remoção de PNG agora usam uma transação de asset com backup e rollback. A preferência completa é salva imediatamente dentro da mesma operação; se o save falha, o arquivo anterior e o identificador anterior são restaurados juntos, com erro em PT-BR e estado de save falho;
- leitura, decode, orientação, resize, reencode e escrita do candidato PNG executam em uma thread nomeada. A main event loop faz somente a troca curta por rename, persistência e atualização AppKit;
- cada métrica mantém geração da importação assíncrona. Resultado obsoleto é descartado, mudança de modo cancela a geração corrente e a UI comunica `Processando PNG…` sem gravar o modo PNG antes do sucesso;
- temporários usam contador por store e tentam o próximo nome em colisões. A regressão cria os nomes reais `.cpu.png.<pid>.<contador>.tmp`;
- o catálogo é uma allowlist explícita com nome persistido, rótulo PT-BR e ano de introdução. O teste cruza a tabela com `CoreGlyphs.bundle/.../name_availability.plist` da Apple e exige ano até 2023, correspondente ao catálogo do macOS 14; o popup mostra rótulo humano e glyph nativo quando o AppKit o fornece;
- o renderer devolve quais identificadores foram realmente resolvidos. Antes de compor as descrições, PNG/SF Symbol ausente vira o mesmo fallback textual desenhado; AX anuncia `fallback textual` e não anuncia o nome do PNG indisponível;
- a descrição visual foi resumida para o estado essencial (`CPU/RAM` + texto/ícone/PNG/fallback), enquanto o label AX preserva cores, identificador resolvido e badge completos. Isso remove a causa do clipping sem empobrecer VoiceOver.

### Ciclos RED → GREEN do rework

- RED: o teste com 32 temporários no padrão real falhou em `result.is_ok()`; GREEN: alocação sequencial pula colisões e preserva os stales.
- RED: `prepare_bytes`/`begin_replace` e o evento de rollback não existiam; GREEN: transações de replace/remove restauram bytes e reducer em falha de persistência.
- RED: reimportar metadados iguais não emitia efeitos, deixando asset ausente/corrompido sem reparo; GREEN: toda importação aceita reinstala o candidato e redesenha.
- RED: mudança de modo durante importação não tinha cancelamento; GREEN: `CancelMetricPngImport` invalida a geração e impede completion fora de ordem.
- RED: preview de PNG ausente ainda continha o nome técnico no AX; GREEN: resolução do renderer alimenta cena de fallback, resumo visual curto e descrição AX fiel.
- RED: catálogo expunha nomes técnicos e o teste chamado “minimum runtime” rodava somente no host atual; GREEN: labels PT-BR/glyphs, allowlist 2019–2023 conferida contra metadata Apple e teste de resolução local renomeado honestamente.

### Limites específicos do rework

- A prova automatizada do catálogo combina allowlist introduzida até 2023 e metadata Apple instalada. A resolução AppKit foi executada no host macOS 26.5.2; este terminal não contém runtime macOS 14 real. O novo QA visual e a validação em runtime mínimo continuam externos a este commit.
- Os artefatos `.superpowers/sdd/2026-08-13-icon-option/visual-qa/` e `visual-qa.md` pertencem ao worker visual e não foram editados nem versionados por este worker.
- O checkpoint precisa de nova avaliação visual com nota mínima 9/10; a rodada 8,5/10 permanece como evidência histórica, não como aprovação.

### Evidências frescas do rework

- `rtk cargo test --test indicator_icon_customization --test indicator_png_flow --test png_icon_assets --bin statlet` — 100 testes focais aprovados em 4 suítes.
- `rtk cargo test` — 246 testes aprovados em 27 suítes.
- `rtk cargo clippy --all-targets -- -D warnings` — sem achados.
- `rtk cargo fmt --check` — aprovado.
- `rtk git diff --check` — aprovado.
- `rtk bash -n scripts/*.sh tests/package_contract.sh` — aprovado.
- `rtk bash tests/package_contract.sh` — build release, bundle arm64/macOS 14+, assinatura ad hoc hardened runtime, ZIP, checksum, privacy manifest e notices aprovados.

## Rework final de concorrência e disco — Task `task_c7bdc63ef1b7`

Os dois P2 do review final foram reproduzidos no checkpoint `6c5781e` e fechados sem editar os artefatos do QA visual concorrente:

- reselecionar outro PNG agora emite cancelamento explícito antes da nova importação; cancelar o `NSOpenPanel` invalida qualquer preparo pendente da métrica; reset de CPU/RAM e reset global cancelam CPU e RAM inclusive quando as preferências já estão nos defaults. O runtime reaproveita o contador por métrica existente, portanto completions da geração anterior são descartados antes de chegar ao reducer;
- a transação de asset passou a depender de uma fronteira de filesystem injetável nos testes. `commit`, rollback e compensações de `begin_replace`/`begin_remove` tentam todas as etapas aplicáveis, agregam falhas de remoção, restauração, cleanup e `fsync`, e nunca descartam silenciosamente o erro. Falha de cleanup depois de salvar preferências permanece observável no estado da métrica sem declarar o save como falho; falha de rollback durante save rejeitado restaura o estado anterior e expõe o detalhe combinado em PT-BR;
- o `Drop` de transação/PNG preparado registra no stderr qualquer cleanup inesperado que não possa mais ser propagado, enquanto os caminhos normais retornam `Result` até o runtime.

### Ciclos RED → GREEN e sensibilidade

- RED: o novo evento de cancelamento ainda não existia; GREEN: 12 testes do fluxo passaram com os quatro casos de invalidar reseleção, cancelamento, reset do grupo e reset global.
- RED: a fronteira `AssetFileSystem` ainda não existia; GREEN: quatro testes com fault injection passaram cobrindo rollback com quatro falhas combinadas, compensação falha após `fsync`, cleanup falho no commit e cleanup falho após escrita temporária parcial.
- Sensibilidade confirmada: neutralizar temporariamente a emissão de cancelamento fez os quatro testes `invalidates` falharem; restaurado o código, 4/4 passaram. Neutralizar temporariamente a agregação de erros fez 3/3 testes de fault injection falharem; restaurado o código, 3/3 passaram.
- O runtime possui testes adicionais para propagação de falha de cleanup após commit e falha de rollback após save rejeitado.

### Evidências frescas

- `rtk cargo test --test indicator_png_flow --test png_icon_assets --bin statlet` — 92 testes aprovados em 3 suítes.
- `rtk cargo test` — 256 testes aprovados em 27 suítes.
- `rtk cargo clippy --all-targets -- -D warnings` — sem achados.
- `rtk cargo fmt --check` — aprovado.
- `rtk git diff --check` — aprovado.
- `rtk bash -n scripts/*.sh tests/package_contract.sh` — aprovado.
- `rtk bash tests/package_contract.sh` — contrato completo do bundle aprovado para arm64/macOS 14+, com assinatura ad hoc hardened runtime, privacy manifest, notices, ZIP e checksum.

## Atomicidade final — Task `task_4a28c6dbc579`

Os dois P2 identificados sobre o checkpoint `7443bf6` foram reproduzidos e corrigidos sem alterações visuais:

- a ação AppKit de modo continua emitindo `SetMetricIdentifierMode` em toda seleção explícita. O reducer agora trata a reseleção do modo já ativo como no-op apenas para preferências, mas ainda emite `CancelMetricPngImport` quando há processamento pendente. O runtime avança a geração e rejeita o completion anterior;
- `PreferencesStore::save` distingue `NotCommitted` de `Committed`. Falhas anteriores ou no próprio rename continuam acionando rollback do asset; falha no `fsync` do diretório após o rename informa que o JSON novo já foi confirmado logicamente;
- no caso pós-rename, o runtime confirma a transação do PNG em vez de restaurar somente o asset, mantém o documento pendente para retry, marca o save como falha de durabilidade e expõe o alerta no erro da métrica. Assim, JSON, estado em memória e PNG permanecem apontando para a mesma versão sem declarar durabilidade inexistente.

### Ciclos RED → GREEN

- RED: `explicitly_reselecting_the_active_mode_cancels_an_in_flight_png_import` recebeu `[]` em vez de `CancelMetricPngImport(Cpu)`; GREEN: a reseleção cancela sem redesenhar nem salvar preferências inalteradas.
- RED: o teste pós-rename não compilava porque o store não classificava commit nem oferecia o ponto de fault injection; GREEN: o fault injection falha exatamente na sincronização do diretório posterior ao rename, retorna `Committed` e `load()` lê o documento novo.
- RED: o runtime não aceitava um resultado de persistência pós-commit distinto; GREEN: `post_rename_preferences_failure_keeps_json_asset_and_runtime_state_aligned` prova JSON novo, PNG novo, estado novo, save `Failed` com documento pendente para retry, transação finalizada e alerta útil.
- Cobertura adicional comprova que falha de rename é `NotCommitted`, remove o temporário e preserva o destino, e que cancelar uma importação invalida a geração usada para filtrar completion obsoleto.

### Evidências frescas

- `rtk cargo test --test indicator_png_flow --test preferences_store --bin statlet` — 92 testes aprovados em 3 suítes.
- `rtk cargo test --lib preferences::location_tests` — 4 testes de classificação/fault injection aprovados.
- `rtk cargo test` — 261 testes aprovados em 27 suítes.
- `rtk cargo clippy --all-targets -- -D warnings` — sem achados.
- `rtk cargo fmt --check` — aprovado.
- `rtk git diff --check` — aprovado.
- `rtk bash -n scripts/*.sh tests/package_contract.sh` — aprovado.
- `rtk bash tests/package_contract.sh` — contrato completo do bundle aprovado para arm64/macOS 14+, com assinatura ad hoc hardened runtime, privacy manifest, notices, ZIP e checksum.

### Limites e preservação

- Não houve alteração de layout, assets ou artefatos do QA visual. A avaliação visual vigente de 9,3/10 continua aplicável ao mesmo layout.
- `.superpowers/sdd/2026-08-13-icon-option/visual-qa.md` e `.superpowers/sdd/2026-08-13-icon-option/visual-qa/` permaneceram não rastreados e intocados.
- Nenhum app foi aberto ou ativado; produção, dados reais, push e PR ficaram fora do escopo.
