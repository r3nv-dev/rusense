# Release Workflow — Plano de Implementação

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Publicar binários prontos do RuSense em GitHub Releases, para que ninguém precise instalar a toolchain Rust só para controlar a ventoinha do notebook.

**Architecture:** Dois artefatos com naturezas opostas, tratados por caminhos separados. O TUI (`rusense`) vira binário **estático musl** — imune a glibc, distro e à aposentadoria de runners. A GUI (`rusense-gui`) **não pode** ser estática (Tauri dlopen'a GL/EGL; `tauri-apps/tauri#5466` está *closed as not planned*), então vai por pacotes nativos `.deb`/`.rpm`. Workflow custom no molde do `casey/just` e do `ripgrep`, sem `cargo-dist`.

**Tech Stack:** GitHub Actions · `x86_64-unknown-linux-musl` · `gh` CLI · Docker (validação de portabilidade) · `tauri-cli` bundler (fase 2)

---

## Contexto verificado (não presumir, já foi medido)

Tudo abaixo foi confirmado empiricamente nesta máquina ou via pesquisa em 2026-09-07:

| Fato | Valor |
|---|---|
| Repositório | `github.com/r3nv-dev/rusense` — público, GPL-3.0 |
| Tag `v0.1.0` | **já existe e já foi enviada** ao remote (aponta `4f79cd7`) |
| Releases publicados | **zero** — a aba Releases está vazia |
| Nome do pacote do TUI | `rusense-tui` ⚠️ **o binário é `rusense`** — `cargo build -p rusense` FALHA |
| TUI em musl | ✅ verificado: `static-pie`, 1,2 MB (984 KB com strip), build em 7,88 s, **sem `musl-gcc`** |
| TUI no Alpine / Rocky 8 | ✅ roda |
| Binário glibc atual no Alpine | ❌ `exec: no such file or directory` |
| GUI | 144 libs dinâmicas (webkit2gtk-4.1, gtk-3, libsoup-3.0) |
| Versão `0.1.0` declarada em | **4 lugares**: `core/`, `tui/`, `gui/Cargo.toml`, `gui/tauri.conf.json` |
| CI atual | `.github/workflows/ci.yml`, roda em `ubuntu-latest` |

### Dois prazos externos que mandam no cronograma

1. **`ubuntu-22.04` entra em deprecação em 2026-09-17** ([runner-images#14254](https://github.com/actions/runner-images/issues/14254)) — brownouts *falham o job*; fim total em 2027-04-17. É o runner que a doc do Tauri ainda recomenda. **O TUI em musl é imune a isso; a GUI não.**
2. **Immutable Releases** estão GA desde 2025-10-28. Se ligadas, assets não podem ser alterados depois de publicar → o padrão *draft → upload → publish* deixa de ser boa prática e vira obrigatório. O plano já nasce nesse formato.

### Decisões tomadas e por quê

- **Sem `cargo-dist`.** Não empacota Tauri, seus defaults de runner (`ubuntu-22.04`, `macos-14`) estão na janela de deprecação, e o bus factor é 1. 12 de 14 CLIs Rust grandes usam workflow custom.
- **Sem AppImage.** Issues abertas quebram exatamente nas suas distros-alvo: [#15976](https://github.com/tauri-apps/tauri/issues/15976) (Fedora 44) e [#15902](https://github.com/tauri-apps/tauri/issues/15902) (Arch/Wayland). E o argumento "roda sem instalar nada" **não se aplica ao RuSense** — o usuário precisa instalar o driver Linuwu-Sense via DKMS de qualquer forma.
- **Fase 1 (TUI) entrega sozinha.** Se a fase 2 travar, o release não fica refém.
- **Lançar `v0.1.1`, não reusar `v0.1.0`.** A tag já é pública; mover tag publicada é má prática mesmo com zero downloads.

---

# FASE 1 — TUI estático ✅ CONCLUÍDA EM 2026-09-07

Publicada como [v0.1.1](https://github.com/r3nv-dev/rusense/releases/tag/v0.1.1).
Tarball de 475 KB, binário estático musl, attestation verificável.

**Dois desvios em relação ao plano escrito, ambos pegos por teste:**

1. `actions/attest` com `predicate: '{}'` é recusado pelo servidor (`build definition is nil`). Para proveniência o correto é `actions/attest-build-provenance`, que monta o predicate SLSA sozinho. A regra "use attest diretamente" vale para attestations customizadas.
2. A tag de teste sugerida (`v0.0.1-test`) não casa com o filtro `v[0-9]+.[0-9]+.[0-9]+` e não dispararia nada. Usar `v0.0.1`.

Além disso, `gui/tauri.conf.json` manteve `version` explícita: com `strip = true` não há como inspecionar o binário e provar que a herança do Cargo.toml funcionou, então o gate confere o campo diretamente.


## Task 1: Unificar a versão em uma fonte única

**Por quê:** hoje a versão está em 4 arquivos. Uma tag `v0.1.1` com os `Cargo.toml` em `0.1.0` publica binário com versão errada e ninguém percebe.

**Files:**
- Modify: `Cargo.toml` (raiz)
- Modify: `core/Cargo.toml`, `tui/Cargo.toml`, `gui/Cargo.toml`

**Step 1: Ver o estado atual**

```bash
cd ~/code/APP-AUTORAL/rusense
grep -n '^version' core/Cargo.toml tui/Cargo.toml gui/Cargo.toml
```
Esperado: três linhas, todas `version = "0.1.0"`.

**Step 2: Declarar a versão no workspace**

No `Cargo.toml` da raiz, adicionar após `resolver = "2"`:

```toml
[workspace.package]
version = "0.1.1"
edition = "2021"
license = "GPL-3.0-only"
```

**Step 3: Fazer os 3 crates herdarem**

Em `core/Cargo.toml`, `tui/Cargo.toml` e `gui/Cargo.toml`, trocar a linha `version = "0.1.0"` por:

```toml
version.workspace = true
```

**Step 4: Verificar que o cargo aceita e que todos leem a mesma versão**

```bash
cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | "\(.name) \(.version)"'
```
Esperado: `rusense-core 0.1.1`, `rusense-tui 0.1.1`, `rusense-gui 0.1.1`.

**Step 5: Confirmar que o build não quebrou**

```bash
cargo build --workspace --release
```
Esperado: `Finished`.

**Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock core/Cargo.toml tui/Cargo.toml gui/Cargo.toml
git commit -m "chore: single source of truth for workspace version (0.1.1)"
```

---

## Task 2: Alinhar a versão do Tauri e reduzir o binário

**Files:**
- Modify: `gui/tauri.conf.json`
- Modify: `Cargo.toml` (raiz)

**Step 1: Testar se o Tauri herda a versão do Cargo.toml**

Remover a linha `"version": "0.1.0",` de `gui/tauri.conf.json` e rodar:

```bash
cargo build --release -p rusense-gui 2>&1 | tail -5
```
Esperado: build passa. O Tauri usa a versão do `Cargo.toml` do crate quando o campo é omitido.

**Se falhar:** restaurar o campo com `"version": "0.1.1"` e anotar que ele precisa de bump manual junto com o workspace. Registrar isso no `README`.

**Step 2: Adicionar strip ao perfil release**

No `Cargo.toml` da raiz:

```toml
[profile.release]
strip = true
```

**Step 3: Medir o ganho**

```bash
cargo build --release -p rusense-tui --target x86_64-unknown-linux-musl
ls -la target/x86_64-unknown-linux-musl/release/rusense
```
Esperado: ~984 KB (era 1.246.992 bytes sem strip).

**Step 4: Commit**

```bash
git add Cargo.toml gui/tauri.conf.json
git commit -m "chore: strip release binaries, inherit tauri version from cargo"
```

---

## Task 3: Script de verificação tag ↔ versão

**Por quê:** nenhuma action popular faz isso (`taiki-e/create-gh-release-action` só parseia a tag; `upload-rust-binary-action` não olha versão). Num workspace é ainda mais fácil dessincronizar.

**Files:**
- Create: `scripts/check-version.sh`

**Step 1: Escrever o script**

```bash
#!/usr/bin/env bash
# Falha se a tag (v0.1.1) não bater com a versão dos crates do workspace.
# Uso:  scripts/check-version.sh v0.1.1
set -euo pipefail

TAG="${1:?uso: check-version.sh <tag>}"
TAG="${TAG#v}"

fail=0
for pkg in rusense-core rusense-tui rusense-gui; do
  V=$(cargo metadata --format-version 1 --no-deps \
      | jq -r --arg p "$pkg" '.packages[] | select(.name==$p) | .version')
  if [ "$V" != "$TAG" ]; then
    echo "::error::$pkg tem versão $V, mas a tag é $TAG"
    fail=1
  else
    echo "ok: $pkg = $V"
  fi
done
exit $fail
```

**Step 2: Tornar executável e testar o caminho FELIZ**

```bash
chmod +x scripts/check-version.sh
scripts/check-version.sh v0.1.1
```
Esperado: três linhas `ok:` e exit 0.

**Step 3: Testar o caminho de FALHA (isto é o teste que importa)**

```bash
scripts/check-version.sh v9.9.9; echo "exit=$?"
```
Esperado: três `::error::` e `exit=1`. **Se sair 0, o script está quebrado** — um gate que nunca falha é pior que nenhum gate.

**Step 4: Commit**

```bash
git add scripts/check-version.sh
git commit -m "ci: add tag/version consistency check"
```

---

## Task 4: Workflow de release — TUI

**Files:**
- Create: `.github/workflows/release.yml`

**Step 1: Obter os SHAs das actions (pinning defensivo)**

Motivo: `tj-actions/changed-files` (CVE-2025-30066) teve **tags reescritas** apontando para commit malicioso; quem pinava por SHA passou ileso.

```bash
for a in actions/checkout actions/upload-artifact actions/attest; do
  echo "$a: $(gh api repos/$a/git/refs/tags --jq '.[-1].object.sha' 2>/dev/null | head -c 40)"
done
```
Anotar os SHAs; usar no formato `uses: actions/checkout@<sha> # v5`.

**Step 2: Escrever o workflow**

```yaml
name: Release

on:
  push:
    tags: ["v[0-9]+.[0-9]+.[0-9]+"]
  workflow_dispatch:          # modo de teste: NÃO cria release
    inputs:
      dry_run:
        description: "Só builda e sobe como artefato do Actions"
        type: boolean
        default: true

permissions:
  contents: read

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<sha>   # v5
      - name: Verificar tag x versão dos crates
        if: startsWith(github.ref, 'refs/tags/')
        run: ./scripts/check-version.sh "${GITHUB_REF_NAME}"

  build-tui:
    needs: verify
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<sha>
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-unknown-linux-musl
      - name: Build estático
        run: cargo build --release --locked -p rusense-tui --target x86_64-unknown-linux-musl
      - name: Provar que é estático
        run: |
          file target/x86_64-unknown-linux-musl/release/rusense
          ldd  target/x86_64-unknown-linux-musl/release/rusense 2>&1 | grep -q "statically linked" \
            || { echo "::error::binário NÃO é estático"; exit 1; }
      - name: Empacotar
        run: |
          V="${GITHUB_REF_NAME:-dev}"
          D="rusense-${V}-x86_64-unknown-linux-musl"
          mkdir "$D"
          cp target/x86_64-unknown-linux-musl/release/rusense "$D"/
          cp README.md LICENSE install.sh 99-rusense.rules "$D"/ 2>/dev/null || true
          tar czf "${D}.tar.gz" "$D"
          sha256sum "${D}.tar.gz" > "${D}.tar.gz.sha256"
      - uses: actions/upload-artifact@<sha>
        with:
          name: tui
          path: rusense-*.tar.gz*

  release:
    needs: build-tui
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    permissions:
      contents: write
      id-token: write
      attestations: write
    steps:
      - uses: actions/checkout@<sha>
      - uses: actions/download-artifact@<sha>
        with: { path: dist, merge-multiple: true }
      - name: Attestation de proveniência
        uses: actions/attest@<sha>       # v4 — NÃO usar attest-build-provenance (virou wrapper)
        with:
          subject-path: dist/*.tar.gz
      - name: Criar release como DRAFT
        env: { GH_TOKEN: "${{ github.token }}" }
        run: |
          gh release create "$GITHUB_REF_NAME" \
            --draft --verify-tag \
            --title "$GITHUB_REF_NAME" \
            --generate-notes \
            dist/*
```

**Nota sobre `--locked`:** obriga o build a respeitar o `Cargo.lock`. Sem isso, um release pode sair com dependências diferentes das testadas.

**Step 3: Validar a sintaxe do YAML antes de commitar**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('YAML ok')"
```
Esperado: `YAML ok`.

**Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow for static musl TUI build"
```

---

## Task 5: Teste local — camadas 1 e 2

**Por quê:** cada ciclo no CI custa minutos. Tudo que dá para provar localmente, prova-se localmente.

**Step 1: Build limpo, igual ao do CI**

```bash
cargo build --release --locked -p rusense-tui --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/rusense
```
Esperado: `static-pie linked`.

**Step 2: Portabilidade real em três distros**

```bash
B="$PWD/target/x86_64-unknown-linux-musl/release"
docker run --rm -v "$B:/b:ro" alpine:latest  /b/rusense --help
docker run --rm -v "$B:/b:ro" debian:11      /b/rusense --help
docker run --rm -v "$B:/b:ro" rockylinux:8   /b/rusense --help
```
Esperado nos três: o texto de uso do `rusense`.

Alpine é o teste decisivo — nem glibc tem. **Não** teste `--once` aqui: sem `/sys` do Acer ele falha por design, e isso não diz nada sobre o binário.

**Step 3: Commit (se algo mudou)**

Nada a commitar se os testes passaram — siga para a Task 6.

---

## Task 6: Teste no CI sem publicar nada

**Step 1: Enviar a branch**

```bash
git push origin master
```

**Step 2: Disparar em modo dry-run**

```bash
gh workflow run release.yml --repo r3nv-dev/rusense -f dry_run=true
sleep 20 && gh run list --repo r3nv-dev/rusense --workflow=release.yml --limit 3
```

**Step 3: Acompanhar até o fim**

```bash
gh run watch --repo r3nv-dev/rusense
```
Esperado: `verify` e `build-tui` verdes; `release` **pulado** (a condição `startsWith(github.ref, 'refs/tags/')` é falsa).

**Step 4: Baixar o artefato e testar o binário que o CI produziu**

```bash
gh run download --repo r3nv-dev/rusense --name tui --dir /tmp/ci-artifact
cd /tmp/ci-artifact && tar xzf rusense-*.tar.gz && cd -
docker run --rm -v /tmp/ci-artifact:/b:ro alpine \
  sh -c 'find /b -name rusense -type f -exec {} --help \;'
```
Esperado: o texto de uso. **Este é o teste que importa** — valida o binário que o CI gerou, não o seu local.

---

## Task 7: Release de mentira, com tag descartável

**Step 1: Criar e enviar uma tag de teste**

```bash
git tag v0.0.1-test && git push origin v0.0.1-test
```

**Step 2: Observar o workflow disparar sozinho**

```bash
gh run watch --repo r3nv-dev/rusense
```

O job `verify` **deve FALHAR** aqui — a tag `0.0.1-test` não bate com `0.1.1`. **Isso é sucesso**: prova que o gate funciona de verdade, não só no teste local da Task 3.

**Step 3: Limpar**

```bash
gh release delete v0.0.1-test --repo r3nv-dev/rusense --yes 2>/dev/null || true
git push --delete origin v0.0.1-test
git tag -d v0.0.1-test
```

---

## Task 8: Release real

**Step 1: Conferir que tudo está commitado e enviado**

```bash
git status --short && git log --oneline -3
```

**Step 2: Taggear e enviar**

```bash
git tag -a v0.1.1 -m "RuSense v0.1.1 — primeiro release com binários"
git push origin v0.1.1
gh run watch --repo r3nv-dev/rusense
```

**Step 3: Inspecionar a DRAFT antes de publicar**

```bash
gh release view v0.1.1 --repo r3nv-dev/rusense
```
Conferir: `.tar.gz` presente, `.sha256` presente, notas geradas.

**Step 4: Baixar e testar como um usuário faria**

```bash
cd /tmp && gh release download v0.1.1 --repo r3nv-dev/rusense --pattern '*.tar.gz'
sha256sum -c rusense-*.sha256 2>/dev/null || echo "conferir checksum manualmente"
tar xzf rusense-*.tar.gz && docker run --rm -v /tmp:/b:ro alpine \
  sh -c 'find /b -name rusense -type f -exec {} --help \;'
```

**Step 5: Publicar**

```bash
gh release edit v0.1.1 --repo r3nv-dev/rusense --draft=false --latest
```

**Step 6: Atualizar o README**

Adicionar uma seção de instalação **antes** da de build, já que agora existe o caminho sem toolchain:

```markdown
## Instalação

### Binário pronto (TUI)

```sh
curl -LO https://github.com/r3nv-dev/rusense/releases/latest/download/rusense-v0.1.1-x86_64-unknown-linux-musl.tar.gz
tar xzf rusense-*.tar.gz && cd rusense-*/
sudo ./install.sh      # regra udev
./rusense
```

Binário estático — não precisa de Rust nem de bibliotecas do sistema. Requer o driver
[Linuwu-Sense](https://github.com/0x7375646F/Linuwu-Sense) carregado.
```

**Step 7: Commit**

```bash
git add README.md && git commit -m "docs: add binary install instructions" && git push
```

**Step 8: Responder ao pedido no PR #132**

O `LFd3v` pediu exatamente isso. Vale avisar no PR que o release existe.

---

# FASE 2 — GUI empacotada (risco alto, escopo separado)

> **Não comece esta fase antes da Fase 1 estar publicada.** A GUI depende de decisões externas com prazo e pode travar; o TUI não pode ficar refém disso.

## Task 9: Decidir o piso de glibc

**A decisão que bloqueia tudo nesta fase.** A GUI é dinâmica, então o runner define quem consegue rodar:

| Opção | glibc mínimo | Quem fica de fora | Risco |
|---|---|---|---|
| `ubuntu-24.04` (= `ubuntu-latest`) | 2.39 | Ubuntu 22.04, Debian 12, RHEL 9 | **nenhum de prazo** |
| `ubuntu-22.04` | 2.35 | Ubuntu 20.04 | ⚠️ **deprecação em 2026-09-17** |
| Container `manylinux_2_28` em runner novo | 2.28 | quase ninguém | build mais complexo |

Fatores: Tauri v2 já exige `gio >= 2.70`, o que **por si só** exclui RHEL 9, Rocky 9 e Ubuntu 20.04 ([tauri#9039](https://github.com/tauri-apps/tauri/issues/9039)). E o público do RuSense é dono de Acer Nitro rodando Arch/Fedora/Ubuntu recente — não servidor antigo.

**Recomendação:** `ubuntu-24.04`. Perde Debian 12, mas evita o prazo e a complexidade de container. Registrar a decisão no README.

## Task 10: Ativar os bundles nativos

**Files:** `gui/tauri.conf.json`

```json
"bundle": {
  "active": true,
  "targets": ["deb", "rpm"],
  "icon": ["icons/icon.png"],
  "linux": {
    "deb": { "depends": ["libwebkit2gtk-4.1-0", "libgtk-3-0", "libsoup-3.0-0"] }
  }
}
```

**Por que declarar `libsoup-3.0` à mão:** o `tauri-cli` injeta automaticamente só `libwebkit2gtk-4.1-0` e `libgtk-3-0`. O soup entra transitivo pelo webkit, mas a GUI linka direto contra ele — declarar evita surpresa.

**Teste local (Arch não gera `.deb` nativamente, então valide no CI ou em container):**

```bash
cargo install tauri-cli --version "^2" --locked
cd gui && cargo tauri build --bundles deb,rpm 2>&1 | tail -20
ls -la target/release/bundle/*/
```

## Task 11: Adicionar o job da GUI ao workflow

Job `build-gui` em `ubuntu-24.04`, com as deps oficiais:

```yaml
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf xdg-utils
```

Depois `cargo tauri build --bundles deb,rpm` e upload dos artefatos de `gui/target/release/bundle/`.

## Task 12: PKGBUILD para o AUR (opcional)

Caminho idiomático para Arch, e o público do RuSense é majoritariamente Arch. Extrai do `.deb`, como a doc do Tauri mostra. `depends` sugerido: `gtk3`, `webkit2gtk-4.1`, `libsoup3`, `cairo`, `pango`, `gdk-pixbuf2`, `glib2`, `hicolor-icon-theme`.

---

## Ordem de execução resumida

```
Fase 1 (faça agora):     Task 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8   ✅ publica v0.1.1
Fase 2 (depois):         Task 9 (decisão) → 10 → 11 → 12
```

## Riscos conhecidos

| Risco | Mitigação |
|---|---|
| Tag errada publica versão errada | Task 3 + gate no `verify`, testado pelo caminho de falha (Task 7) |
| Binário do CI diferente do local | Task 6 baixa o artefato do CI e testa em container |
| Action comprometida via tag reescrita | Pinning por SHA (Task 4, Step 1) |
| Release publicado com asset faltando | Nasce como draft; só publica após inspeção (Task 8) |
| `ubuntu-22.04` deprecado | Fase 1 não usa; Fase 2 decide em Task 9 |
| AppImage quebrado em Arch/Fedora | Fora de escopo por decisão |
