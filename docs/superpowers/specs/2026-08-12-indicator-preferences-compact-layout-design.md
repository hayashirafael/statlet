# Layout compacto das preferências do indicador

Status: aprovado em 12 de agosto de 2026.

## Problema

A tela do indicador usa um canvas com coordenadas fixas. Cada editor de cor ocupa 160 pt mesmo quando está oculto, então os modos dinâmicos deixam grandes vazios entre CPU, RAM e os grupos seguintes. O botão **Restaurar CPU e RAM** também aparece ao lado do controle da CPU, apesar de afetar as duas métricas.

## Direção aprovada

CPU e RAM formam um único grupo visual chamado **Cores**:

- o cabeçalho do grupo mostra **Cores** à esquerda e **Restaurar CPU e RAM** à direita;
- CPU e RAM são linhas da mesma grade, com rótulos de largura igual e seletores alinhados;
- cada editor de cor aparece imediatamente abaixo da respectiva linha somente no modo **Fixa**;
- ocultar um editor remove sua altura do fluxo, sem deixar espaço reservado;
- o espaçamento interno fica entre 12 e 16 pt; grupos vizinhos mantêm 24 pt entre si.

Os grupos **Rótulos**, **Tipografia** e **Atualização** entram no mesmo fluxo vertical. Controles com a mesma função compartilham coluna e baseline. Os botões de restauração pertencem ao cabeçalho ou rodapé do grupo que realmente alteram, nunca a uma linha com escopo menor.

## Comportamento dinâmico

O layout é recalculado quando o modo de CPU, RAM ou rótulos muda. A altura de cada grupo deriva apenas dos controles visíveis. A rolagem preserva a posição atual sempre que possível e o documento rolável informa sua nova altura ao AppKit.

Essa mudança afeta somente a organização da janela. Preferências, eventos, ordem de teclado, restauração, persistência, renderer e coleta de métricas mantêm seus contratos atuais.

## Implementação

Uma função pura calcula a geometria vertical a partir de três estados de visibilidade: cor fixa da CPU, cor fixa da RAM e cor personalizada dos rótulos. Ela devolve os frames dos grupos e a altura total do documento. A camada AppKit apenas aplica esses frames aos controles existentes.

Essa fronteira permite testar as combinações sem depender de uma sessão gráfica e evita espalhar novas constantes de coordenadas pela janela.

## Testes e validação

Os testes automatizados devem provar que:

1. o modo totalmente dinâmico produz o layout mais compacto;
2. cada editor fixo acrescenta exatamente sua própria altura e desloca somente o conteúdo posterior;
3. todas as combinações preservam o espaçamento definido e não sobrepõem controles;
4. CPU e RAM usam a mesma coluna para rótulos e seletores;
5. o botão de restauração permanece associado ao grupo combinado;
6. a ordem de teclado continua acompanhando apenas os controles visíveis.

A validação manual deve abrir a janela no macOS, alternar CPU, RAM e rótulos entre todos os modos e conferir alinhamento, rolagem, foco e ausência de saltos ou grandes vazios.

## Fora do escopo

- alterar cores, fontes, intervalos ou defaults;
- mudar a prévia fixa, o rodapé ou a área **Disco e Mole**;
- redesenhar a janela inteira com outro framework;
- modificar a apresentação do indicador na menu bar.

## Compatibilidade

O design preserva o ADR 0001: não adiciona polling, timers, workers ou trabalho recorrente. O recálculo acontece somente em alterações explícitas de preferência e permanece restrito à janela.
