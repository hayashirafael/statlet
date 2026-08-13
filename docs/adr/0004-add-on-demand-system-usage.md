---
status: accepted
---

# Adicionar Uso do sistema como extensão sob demanda

O Statlet adicionará uma janela nativa **Uso do sistema** com detalhes vivos de RAM e GPU sem transformar a experiência em uma terceira métrica permanente. A janela reutiliza o tick principal de dois segundos, mantém no máximo 150 pontos somente em memória e não grava séries ou processos no **Histórico de atividade**.

A RAM detalhada preserva exatamente a fórmula do indicador compacto: apps, memória reservada pelo sistema e memória comprimida sobre a memória física; cache recuperável e swap permanecem informações separadas. A GPU é lida diretamente de `PerformanceStatistics` em serviços `AGXAccelerator*` pelo IOKit somente enquanto a janela está visível. Essas chaves são tratadas como capacidade best-effort: ausência, mudança ou tipo inválido torna apenas a GPU indisponível.

## Consequências

- O indicador compacto, a cadência de disco e o histórico persistido da v1 permanecem inalterados.
- Não são criados timer, thread permanente, subprocesso, elevação ou persistência adicional.
- Top processos RAM é efêmero, limitado a 20 e coletado apenas com a janela aberta.
- Falhas e sleep/wake geram lacunas sem interpolação ou catch-up; a última leitura pode ser preservada somente quando identificada como antiga.
- A aceitação de release exige os gates v1 com a janela fechada e um soak comparativo com a janela aberta, além de QA manual AppKit/VoiceOver.
