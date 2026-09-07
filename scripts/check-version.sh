#!/usr/bin/env bash
# Falha se a tag do release não bater com a versão declarada no projeto.
#
# A versão vive em [workspace.package] no Cargo.toml raiz (herdada pelos três
# crates) e, separadamente, em gui/tauri.conf.json — que o bundler usa para
# nomear .deb/.rpm. Os dois precisam casar com a tag, senão o release sai com
# binário e pacote anunciando versões diferentes.
#
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

TV=$(jq -r '.version // empty' gui/tauri.conf.json)
if [ -z "$TV" ]; then
  echo "::error::gui/tauri.conf.json não declara version"
  fail=1
elif [ "$TV" != "$TAG" ]; then
  echo "::error::gui/tauri.conf.json tem versão $TV, mas a tag é $TAG"
  fail=1
else
  echo "ok: tauri.conf.json = $TV"
fi

exit $fail
