#!/usr/bin/env bash
# RuSense — alerta de bateria baixa.
# Lê a telemetria via `rusense --once` e dispara UMA notificação por
# descida abaixo do limiar; o estado reseta quando a bateria volta a
# carregar ou sobe acima do limiar. Pensado para rodar por um timer
# systemd de usuário (ver dist/systemd/).
#
# Variáveis:
#   RUSENSE_BATTERY_THRESHOLD  limiar em % (padrão: 20)
#   RUSENSE_BIN                caminho do binário rusense (padrão: ~/.cargo/bin/rusense)
set -euo pipefail

THRESHOLD="${RUSENSE_BATTERY_THRESHOLD:-20}"
STATE="${XDG_RUNTIME_DIR:-/tmp}/rusense-battery-alert.notified"
RUSENSE="${RUSENSE_BIN:-$HOME/.cargo/bin/rusense}"

json="$("$RUSENSE" --once)" || exit 0
pct="$(sed -E 's/.*"battery_pct":([0-9]+).*/\1/' <<<"$json")"
status="$(sed -E 's/.*"battery_status":"([^"]*)".*/\1/' <<<"$json")"

if [[ "$status" == "Discharging" && "$pct" -le "$THRESHOLD" ]]; then
    if [[ ! -f "$STATE" ]]; then
        notify-send -u critical -a RuSense "Bateria baixa" \
            "${pct}% restante — conecte o carregador."
        touch "$STATE"
    fi
else
    rm -f "$STATE"
fi
