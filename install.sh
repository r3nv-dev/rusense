#!/usr/bin/env bash
# install.sh — instala a regra udev do RuSense.
#
# A regra (dist/99-rusense.rules) dá ao grupo "wheel" permissão de escrita
# nos controles do driver Linuwu-Sense (nitro_sense/predator_sense e
# platform-profile). Sem ela, rusense/rusense-gui funcionam em modo SOMENTE
# LEITURA — o próprio app avisa: "sem permissão de escrita — rode o
# install.sh (regra udev)".
set -euo pipefail

RULE_SRC="$(dirname "$(readlink -f "$0")")/dist/99-rusense.rules"
RULE_DST="/etc/udev/rules.d/99-rusense.rules"
DRIVER_BASE="/sys/devices/platform/acer-wmi"

if [[ ! -d "$DRIVER_BASE/nitro_sense" && ! -d "$DRIVER_BASE/predator_sense" ]]; then
    echo "erro: driver Linuwu-Sense não encontrado em $DRIVER_BASE." >&2
    echo "Instale e carregue o driver antes de rodar este script:" >&2
    echo "  https://github.com/0x7375646F/Linuwu-Sense" >&2
    exit 1
fi

if [[ ${EUID} -ne 0 ]]; then
    echo "erro: este script precisa de root pra escrever em /etc/udev/rules.d/." >&2
    echo "Rode: sudo ./install.sh" >&2
    exit 1
fi

# Aviso não-fatal: em Debian/Ubuntu não existe grupo "wheel" — a regra
# instala mesmo assim, mas só faz efeito depois de trocar o grupo nela.
if ! getent group wheel >/dev/null 2>&1; then
    echo "aviso: o grupo 'wheel' não existe nesta distro (comum em Debian/Ubuntu)." >&2
    echo "  A regra será instalada mesmo assim, mas sem efeito até você trocar" >&2
    echo "  'wheel' por um grupo admin real (ex.: 'sudo') em $RULE_DST" >&2
    echo "  e recarregar: sudo udevadm control --reload && sudo udevadm trigger \\" >&2
    echo "    --action=add --subsystem-match=platform --sysname-match=acer-wmi" >&2
fi

install -m 644 "$RULE_SRC" "$RULE_DST"
udevadm control --reload
# --action=add é essencial: o trigger sintetiza eventos CHANGE por padrão,
# que não casam com o ACTION=="add" da regra — sem ele, as permissões só
# seriam aplicadas no próximo boot/reload do driver. --sysname-match limita
# o add sintético ao acer-wmi em vez de re-disparar todo device platform.
udevadm trigger --action=add --subsystem-match=platform --sysname-match=acer-wmi

echo "feito:"
echo "  - regra copiada pra $RULE_DST"
echo "  - udev recarregado e device platform re-disparado (permissões já aplicadas)"
echo "  - grupo 'wheel' agora tem escrita nos controles do driver"
echo
echo "pra desfazer:"
echo "  sudo rm $RULE_DST"
echo "  sudo udevadm control --reload"
echo "  (remover a regra não revoga permissões já aplicadas — o modo somente"
echo "   leitura volta no próximo boot ou reload do driver linuwu_sense)"
