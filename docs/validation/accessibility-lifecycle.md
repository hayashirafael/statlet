# Validação de acessibilidade e ciclo de vida

Checklist da v1 para os comportamentos que dependem de AppKit, configurações do macOS ou hardware real. Os contratos automatizáveis permanecem na suíte de testes.

Este documento preserva a evidência da v1.0.0. A personalização posterior do indicador possui um checklist separado, em [`indicator-customization.md`](indicator-customization.md); itens manuais desse novo fluxo não herdam o estado concluído da validação v1.

## Cobertura automatizada

- [x] Launch direto pede ao core uma janela útil de Preferências.
- [x] Reopen sem janela visível pede ao core a mesma janela reutilizável.
- [x] Reopen com janela visível não cria outra janela.
- [x] Um intervalo de sleep reinicia o debounce e não completa cinco minutos sem observações.
- [x] Um episódio já ativo atravessa sleep sem novo alerta e registra uma única recuperação.
- [x] O agendador executa uma amostra após wake, sem rajada para compensar minutos perdidos.

## Inspeção de implementação

- [x] Checkbox, seletor e botões são controles AppKit nativos, com rótulos e ajuda de acessibilidade explícitos nas ações que precisam de contexto.
- [x] Valores de disco e estado do Mole expõem rótulos completos, não pares de textos sem relação.
- [x] Linhas do histórico têm descrição completa e ordem indicada no contêiner rolável.
- [x] O indicador descreve CPU, RAM, pressão de memória e disco em um único rótulo acessível.
- [x] O badge usa `!` para atenção e `×` para bloqueio além das cores.
- [x] Texto e controles usam cores semânticas do sistema, que fornecem variantes para Light, Dark e Increase Contrast.
- [x] Cada tipo de janela é retido em um único slot e atualizado antes de voltar à frente.
- [x] Falha ao criar o item da menu bar não encerra o app nem impede o evento que abre Preferências.

## Execução manual e validações residuais

- [x] Abrir o bundle local com Launch Services mostra Preferências mesmo sem interação com o item da menu bar.
- [x] Conferir Preferências em Light Mode: conteúdo, estados desabilitados e hierarquia permanecem legíveis.
- [x] Conferir Preferências em Dark Mode: conteúdo, estados desabilitados e hierarquia permanecem legíveis.
- [x] Inspecionar a árvore de acessibilidade da janela real: checkbox, seletor, rótulos e estados habilitado/desabilitado foram expostos; `AXPress` alternou a preferência e a integração foi restaurada para desativada.
- [ ] Com Full Keyboard Access ativo, percorrer checkbox, seletor, botão do Mole e botão de apagar histórico; confirmar foco visível e acionamento por teclado.
- [ ] Com VoiceOver, percorrer indicador, Preferências, Liberar espaço, Histórico e o alerta de confirmação; confirmar papel, nome, valor, ajuda e ordem.
- [ ] Em Increase Contrast, conferir indicador e as três janelas em Light e Dark.
- [ ] Com dois monitores, mover uma janela, desconectar o monitor e reabrir cada entrada; confirmar uma única janela visível e estado preservado.
- [ ] Fechar Preferências com o app ainda rodando e reabrir o bundle pelo Finder e pelo Spotlight; confirmar reutilização e foco.
- [ ] Suspender o Mac durante debounce e durante episódio ativo; após wake, confirmar ausência de alerta falso ou duplicado.

Os itens abertos acima registram o limite real do ambiente de release — macOS 26.5.2, um único display interno e sem sessão de validação assistida por VoiceOver — e não são alegações de execução. Os contratos automatizados cobrem as transições de estado; hardware e tecnologias assistivas ainda pedem uma rodada manual dedicada.

Na validação da personalização de 12 de agosto de 2026, o Statlet não foi aberto nem ativado para inspeção visual porque não havia autorização de foreground. Light/Dark, controles nativos, tecnologias assistivas, notch, Retina, troca de display e sleep/wake permanecem explicitamente não executados no checklist específico; o soak autorizado inicia somente o executável do bundle em background, sem interação de UI, e não conta como validação visual ou acessível.

As verificações seguem as orientações da Apple para [acessibilidade](https://developer.apple.com/design/human-interface-guidelines/accessibility/), [cores semânticas](https://developer.apple.com/design/human-interface-guidelines/color/) e [interação por teclado no macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos/).
