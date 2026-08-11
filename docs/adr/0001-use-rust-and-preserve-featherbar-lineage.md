---
status: accepted
---

# Usar Rust e preservar a linhagem do featherbar

O Statlet começará como uma derivação rastreável do featherbar em Rust porque ele já demonstra o layout essencial de duas linhas em um único `NSStatusItem` com baixo overhead. O histórico, copyright, licença Apache 2.0 e avisos aplicáveis serão preservados; antes da distribuição, o código e as obrigações de licença serão revalidados.

## Consequências

A implementação deve provar que continua leve em Apple Silicon e evitar abstrações ou features que descaracterizem o modelo de performance da referência.
