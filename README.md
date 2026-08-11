# Statlet

> Tiny system stats, stacked.

Statlet será um monitor open source, compacto e local para a menu bar do macOS. CPU e RAM aparecem simultaneamente em duas linhas dentro de um único item:

```text
C 18%
R 63%
```

O projeto está em fase de especificação. Ainda não há código, release ou arquitetura implementada.

## Proposta

- leitura imediata de CPU e RAM sem abrir um dashboard;
- largura fixa e reduzida, adequada a MacBooks com notch;
- funcionamento local, sem telemetry;
- baixo consumo de CPU, memória e energia;
- nenhuma permissão administrativa para as métricas principais;
- integração opcional com o [Mole](https://github.com/tw93/Mole) para avisos de pouco espaço.

## Experiência principal

CPU e RAM ficam sempre visíveis, nessa ordem, no preset compacto. O clique abre um menu nativo com acesso a preferências, histórico e, quando habilitado, ao estado do disco.

- CPU: uso global normalizado entre 0 e 100%;
- RAM: memória de apps, wired e comprimida sobre a memória física, sem contar cache recuperável;
- atualização de CPU e RAM: a cada 2 segundos;
- cor da CPU: verde de 0–39%, laranja de 40–69% e vermelho de 70–100%;
- cor da RAM: segue o memory pressure do macOS.

## Disco e Mole

O disco é opcional. Seu badge só aparece quando a pessoa ativa a integração com o Mole nas preferências.

- monitora somente o volume de inicialização;
- verifica o disco a cada 60 segundos;
- limite padrão de 90% usado, configurável de 70% a 95% em passos de 5%;
- alerta somente após 5 minutos continuamente acima do limite;
- emite uma notificação por episódio e rearma após voltar abaixo do limite;
- considera como disponível o espaço que o macOS pode recuperar automaticamente;
- nunca instala o Mole automaticamente.

Ultrapassar o limite **não inicia uma limpeza**. A notificação ou a opção **“Revisar espaço…”** abre a janela **“Liberar espaço”**.

O Mole atual ainda não oferece ao Statlet um plano estruturado e reproduzível, nem uma garantia pública de execução somente no escopo do usuário e sem aproveitar uma sessão `sudo` existente. Por isso, a v1 não executará limpeza dentro do app. Enquanto esse contrato não existir, a janela orientará a pessoa a abrir o fluxo oficial do Mole no Terminal.

## Segurança

Uma futura limpeza integrada só será habilitada se o Mole oferecer:

1. plano versionado e legível por máquina;
2. escopo explicitamente restrito ao usuário, sem `sudo`;
3. execução vinculada ao plano revisado;
4. progresso e resultado estruturados;
5. revalidação do plano imediatamente antes da remoção.

Até lá, o Statlet não analisará prompts, ANSI ou arquivos textuais internos do Mole como se fossem uma API estável.

## Histórico

O Statlet manterá localmente os 30 eventos mais recentes, sem nomes ou caminhos de arquivos.

Na v1, o histórico cobre alertas e bloqueios da integração. Quando a execução segura dentro do app existir, também poderá registrar horário, duração, uso antes/depois, espaço liberado e resultado. Uma limpeza conduzida externamente no Terminal não será atribuída ao Statlet.

## Direção técnica

A implementação inicial será em Rust e partirá de uma derivação rastreável do [featherbar](https://github.com/nim444/featherbar), preservando histórico, copyright, licença Apache 2.0 e avisos aplicáveis.

O objetivo de performance segue a referência:

- um event loop principal;
- nenhuma worker thread permanente enquanto o app estiver ocioso;
- estado de renderização reutilizado;
- amostragem mínima;
- `autoreleasepool` por ciclo;
- build de release otimizado e medição prolongada em Apple Silicon.

Isso é uma direção aprovada, não uma afirmação de performance ainda não medida.

## Distribuição

A v1 será distribuída diretamente, assinada e notarizada com Developer ID. O canal inicial será GitHub Releases, com Homebrew Cask após estabilização do pacote. A Mac App Store poderá ser reavaliada se a integração externa se tornar compatível com App Sandbox.

## Licença

Statlet é distribuído sob a licença [Apache 2.0](LICENSE). Código derivado do featherbar preservará os avisos e atribuições do projeto original.

## Documentação

- [Contrato de produto da v1](docs/product/v1.md)
- [Linguagem do domínio](CONTEXT.md)
- [Decisões arquiteturais](docs/adr/)
- [Pesquisa de UI/UX e mercado](docs/research/disk-cleanup-ui-market.md)

## Antes de implementar

- validar featherbar e métricas em um Mac Apple Silicon;
- medir CPU, memória, energia e wakeups do próprio Statlet;
- testar legibilidade em Retina, Light/Dark Mode, Increase Contrast e VoiceOver;
- validar sleep/wake, troca de monitor e execução prolongada;
- revalidar licença e comportamento das versões compatíveis do Mole;
- definir o deployment target mínimo do macOS.

Nenhum comportamento destrutivo será implementado apenas para completar a interface.
