---
status: accepted
---

# Distribuir a v1 diretamente

A v1 será assinada e notarizada com Developer ID e distribuída por GitHub Releases, com Homebrew Cask após estabilização. A distribuição direta foi escolhida porque localizar e executar um Mole instalado externamente e permitir sua varredura é um risco arquitetural alto sob App Sandbox; a Mac App Store poderá ser reconsiderada se essa fronteira mudar.

## Gate externo da primeira publicação

Developer ID e notarização continuam sendo o estado de distribuição aprovado. Se as credenciais externas não estiverem configuradas, uma publicação inicial pode levar apenas uma assinatura ad hoc de hardened runtime desde que declare de forma proeminente que o Gatekeeper poderá bloqueá-la, ofereça checksum e build verificável da fonte e nunca recomende remover quarentena ou desativar o Gatekeeper. A ausência de credenciais não autoriza reduzir as verificações do pipeline nem muda a decisão de distribuição direta.
