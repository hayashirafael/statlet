# Statlet

Linguagem comum do produto Statlet. Este arquivo define os termos do domínio, sem prescrever como serão implementados.

## Language

**Indicador compacto**:
Representação permanente de CPU e RAM em duas linhas dentro de um único item da menu bar.
_Avoid_: Widget, dashboard, preset completo

**Integração com o Mole**:
Opção ativada pela pessoa que conecta os avisos de disco do Statlet ao fluxo de manutenção oferecido pelo Mole.
_Avoid_: Limpador automático, engine de limpeza do Statlet

**Badge de disco**:
Sinal visual anexado ao indicador compacto para comunicar atenção, atividade, conclusão ou bloqueio relacionados ao disco. Só existe quando a integração com o Mole está ativada.
_Avoid_: Terceira métrica permanente

**Espaço disponível**:
Capacidade que pode ser usada para uma operação importante, incluindo espaço que o macOS consegue recuperar automaticamente.
_Avoid_: Espaço livre

**Limite de aviso**:
Percentual configurado de ocupação do volume de inicialização a partir do qual um episódio pode começar.
_Avoid_: Limite de limpeza

**Episódio de pouco espaço**:
Período iniciado após o volume permanecer cinco minutos acima do limite de aviso e encerrado quando volta abaixo dele.
_Avoid_: Evento de limpeza, alerta por amostra

**Revisão de espaço**:
Experiência iniciada pela pessoa para entender o estado do disco e escolher como prosseguir. Abrir essa experiência nunca remove arquivos.
_Avoid_: Limpeza automática

**Contrato seguro do Mole**:
Interface versionada que vincula um plano revisável a uma execução restrita ao usuário, sem `sudo`, com revalidação e resultados estruturados.
_Avoid_: Parsing de terminal, dry-run textual

**Histórico de atividade**:
Registro local e resumido dos eventos observados pelo Statlet, sem nomes ou caminhos de arquivos.
_Avoid_: Log do Mole, relatório de arquivos
