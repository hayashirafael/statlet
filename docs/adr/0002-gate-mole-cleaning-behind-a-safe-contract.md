---
status: accepted
---

# Bloquear limpeza pelo Mole até existir um contrato seguro

O Statlet apenas monitora, notifica e encaminha a pessoa ao fluxo oficial do Mole. Nenhuma limpeza será executada dentro do app enquanto o Mole não fornecer um plano estruturado, vinculado à execução e explicitamente restrito ao usuário sem `sudo`; essa escolha substitui a proposta anterior de limpeza automática e evita que uma credencial `sudo` cacheada amplie silenciosamente o escopo.

## Consequências

A janela pode explicar e orientar, mas não deve simular uma integração que o backend não consegue honrar. Relatórios de limpeza só serão gerados para execuções observadas pelo Statlet sob esse contrato.
