---
status: accepted
---

# Usar Rust e preservar a linhagem do featherbar

O Statlet começará como uma derivação rastreável do featherbar em Rust porque ele já demonstra o layout essencial de duas linhas em um único `NSStatusItem` com baixo overhead. O histórico, copyright, licença Apache 2.0 e avisos aplicáveis serão preservados; antes da distribuição, o código e as obrigações de licença serão revalidados.

## Consequências

A implementação deve provar que continua leve em Apple Silicon e evitar abstrações ou features que descaracterizem o modelo de performance da referência.

O protótipo `prototype/runtime-feasibility`, baseado no featherbar `90ab504b025db15665ce5d97b8ae4d4cdeb47dc3`, confirmou a viabilidade no Apple M4. Ele também mostrou que “sem workers permanentes” descreve threads criadas pelo Statlet, não as threads internas que AppKit e libdispatch adicionam ao processo.
