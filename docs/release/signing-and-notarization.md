# Assinatura e notarização

O Statlet usa Developer ID e o serviço notarial da Apple quando as credenciais estão configuradas. O script não reduz a verificação quando elas estão ausentes: a build local recebe apenas uma assinatura ad hoc de hardened runtime, e a release precisa declarar esse estado.

## Gate externo atual

Verificado em 11 de agosto de 2026:

- `security find-identity -v -p codesigning` encontrou **0 identidades válidas**;
- o repositório `hayashirafael/statlet` não possui secrets configurados para Apple Developer;
- portanto, esta máquina não consegue produzir nem notarizar uma assinatura Developer ID.

Para liberar o gate, o mantenedor precisa fornecer:

1. participação ativa no Apple Developer Program;
2. certificado **Developer ID Application** e chave privada instalados no Keychain;
3. credenciais salvas pelo `notarytool`, por exemplo com uma chave da App Store Connect ou Apple ID, senha específica de app e Team ID;
4. `STATLET_SIGNING_IDENTITY` com o nome exato da identidade;
5. `STATLET_NOTARY_PROFILE` com o nome do perfil salvo no Keychain.

## Processo verificável

```bash
export STATLET_SIGNING_IDENTITY='Developer ID Application: ...'
export STATLET_NOTARY_PROFILE='statlet-notary'
bash scripts/package-release.sh dist
bash scripts/notarize-release.sh dist
```

`package-release.sh` usa hardened runtime e timestamp ao encontrar a identidade. `notarize-release.sh` exige as duas variáveis, aguarda o resultado da Apple, grampeia o ticket no app, valida com `stapler`, avalia com Gatekeeper (`spctl`) e só então recria o ZIP e o SHA-256.

Verificação independente:

```bash
codesign --verify --deep --strict --verbose=2 dist/Statlet.app
xcrun stapler validate dist/Statlet.app
spctl --assess --type execute --verbose=4 dist/Statlet.app
```

Como o checksum usa somente o nome do arquivo para funcionar após o download, execute a última validação dentro do diretório dos artefatos:

```bash
(cd dist && shasum -a 256 -c Statlet-v1.0.0-macos-arm64.zip.sha256)
```

Não é aceitável instruir a pessoa a desativar o Gatekeeper ou remover a quarentena para mascarar a ausência de notarização.
