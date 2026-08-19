# Validação do menu e da visibilidade — 2026-08-19

## Resultado

A implementação das opções M-A e V-A foi aprovada em revisão independente, sem findings P1/P2, e validada em um bundle Dev isolado. O menu apresenta estado não interativo, ações e comandos do app em grupos distintos; a página Geral controla somente a presença do status item; ocultar mantém o processo vivo; reabrir o bundle recupera Preferências; e os atalhos nativos funcionam.

Esta rodada não alterou nem encerrou o Statlet de produção. A instância de produção permaneceu no PID 55442, executando `/Users/user/Developer/projects/statlet/dist/Statlet.app/Contents/MacOS/Statlet`, enquanto o QA usou identidade, bundle ID e armazenamento Dev próprios.

## Evidência automatizada

- `rtk cargo fmt --all -- --check`: exit 0.
- `rtk cargo test --all-targets --all-features --locked`: 416 testes aprovados em 29 suítes após a correção do QA.
- `rtk cargo clippy --all-targets --all-features --locked -- -D warnings`: nenhum finding.
- `rtk git diff --check`: exit 0.
- `rtk bash -n scripts/*.sh tests/*.sh`: exit 0.
- `rtk bash tests/dev_package_contract.sh`: aprovado, incluindo bundles Dev distintos, arquitetura arm64, macOS 14+, assinatura ad-hoc com hardened runtime e destinos seguros.
- Revisão read-only integral: **Standards Approved** e **Spec Approved**, sem P1/P2.

## Artefato observado

- bundle: `/private/var/folders/rd/c53m2w5j25g68bkx2zdw_mfh0000gp/T/statlet-ux-qa-final.XXXXXX.nS0w7KM624/Statlet Dev 09AA.app`;
- bundle ID: `io.github.hayashirafael.Statlet.dev.ux-review-macos-2026-08-09aaaa5e91bb`;
- instância: `ux-review-macos-2026-08-09aaaa5e91bb`;
- executável SHA-256: `2907790440105787171a0337306afb77e5150596a86ab89dd626258345b2c333`;
- `scripts/verify-dev-bundle.sh`: versão 1.0.0, arm64, macOS 14+, assinatura ad-hoc e hardened runtime aprovados;
- ambiente observado: Apple M4, macOS 26.5.2, aparência escura, display Retina interno.

O primeiro pacote feito com o seed longo expôs um defeito no script: o slug truncado terminava em hífen, gerava um ID com hífen duplo e era rejeitado pelo runtime em `src/main.rs:597`. O script e seu contrato foram corrigidos em TDD; o bundle listado acima usa o mesmo seed longo e iniciou normalmente.

## Evidência nativa observada

- A janela de Preferências abriu em 860 × 732 com **Geral** selecionado.
- A árvore AX expôs a sidebar com seis áreas, o checkbox **Mostrar o Statlet na barra de menus**, valor e ajuda completos.
- O texto de recuperação ficou integralmente visível em duas linhas após a correção de wrapping.
- `Tab` moveu o foco da sidebar para o checkbox.
- O status item expôs descrição acessível com identidade Dev, CPU, RAM e pressão de memória.
- O menu apresentou uma linha de CPU/RAM desabilitada, separadores, **Uso do sistema…**, **Revisar espaço…** desabilitado no estado observado, **Preferências…**, **Histórico…**, identidade Dev desabilitada e **Sair**.
- O menu principal expôs Sobre, Serviços, Ocultar, Ocultar Outros, Mostrar Tudo e os atalhos nativos `⌘,`, `⌘Q` e `⌘W`.
- Desmarcar a opção removeu o segundo menu bar do processo, persistiu `showInMenuBar: false` e manteve o processo vivo.
- `⌘W` fechou Preferências sem encerrar o app.
- Reabrir o bundle via Launch Services recriou Preferências com o checkbox ainda desmarcado; remarcar recriou imediatamente um único status item.
- `⌘,` reabriu Preferências após `⌘W`; `⌘Q` encerrou apenas a instância Dev.
- Ao final, nenhum processo Dev permaneceu ativo e a instância de produção continuou no mesmo PID e executável.

## Gates ainda abertos

Os seguintes cenários não foram executados porque exigiriam mudar preferências globais, interromper a sessão ou dispor de hardware adicional:

- percurso assistido com VoiceOver e Accessibility Inspector;
- percurso completo com Full Keyboard Access ativo;
- Increase Contrast em Light e Dark;
- Finder e Spotlight operados pela interface, separadamente;
- escala alternativa, monitor externo e desconexão de display;
- sleep/wake físico com o status item visível e oculto.

Esses gates estão especificados na [issue #23](https://github.com/hayashirafael/statlet/issues/23), marcada `ready-for-human`. Eles permanecem desconhecidos; a evidência AX e os testes automatizados não os substituem.

## Fontes normativas

A solução segue as fontes primárias da Apple para [teclado](https://developer.apple.com/design/human-interface-guidelines/keyboards), [menus](https://developer.apple.com/design/human-interface-guidelines/menus), [ajustes](https://developer.apple.com/design/human-interface-guidelines/settings), [acessibilidade](https://developer.apple.com/design/human-interface-guidelines/accessibility/) e [`NSStatusBar`](https://developer.apple.com/documentation/appkit/nsstatusbar?changes=la).
