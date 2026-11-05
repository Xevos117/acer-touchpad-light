#!/usr/bin/env bash
# Build tplight as the current user and install it system-wide (uses sudo).
#
#   ./install.sh              build + install binary, config (if absent) and systemd unit
#   ./install.sh --udev       additionally install the optional udev rule (group "tplight")
#   ./install.sh --uninstall  stop and remove everything (keeps /etc config unless --purge)
#   ./install.sh --uninstall --purge
set -euo pipefail

BIN=/usr/local/bin/tplight
CONF_DIR=/etc/acer-touchpad-light
UNIT=/etc/systemd/system/tplight.service
RULE=/etc/udev/rules.d/70-acer-touchpad-light.rules
cd "$(dirname "$0")"

if [[ ${EUID} -eq 0 ]]; then
    echo "Run this script as your normal user; it calls sudo only for the install step." >&2
    exit 1
fi

uninstall() {
    sudo systemctl disable --now tplight.service 2>/dev/null || true
    # make sure no effect keeps looping
    [[ -x ${BIN} ]] && sudo "${BIN}" stop 2>/dev/null || true
    sudo rm -f "${BIN}" "${UNIT}" "${RULE}"
    sudo systemctl daemon-reload
    sudo udevadm control --reload 2>/dev/null || true
    if [[ ${1:-} == "--purge" ]]; then
        sudo rm -rf "${CONF_DIR}"
    else
        echo "Configuration kept in ${CONF_DIR} (use --uninstall --purge to remove it)."
    fi
    echo "Uninstalled."
}

UDEV=0
case "${1:-}" in
    --uninstall) uninstall "${2:-}"; exit 0 ;;
    --udev) UDEV=1 ;;
    "") ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
esac

CARGO=${CARGO:-$(command -v cargo || echo "${HOME}/.cargo/bin/cargo")}
[[ -x ${CARGO} ]] || { echo "cargo not found (install Rust from https://rustup.rs)" >&2; exit 1; }

echo "==> Building (release) as ${USER}"
"${CARGO}" build --release --locked

echo "==> Installing (sudo)"
sudo install -Dm755 target/release/tplight "${BIN}"
if [[ -e ${CONF_DIR}/config.toml ]]; then
    echo "    keeping existing ${CONF_DIR}/config.toml"
else
    sudo install -Dm644 config/config.toml "${CONF_DIR}/config.toml"
fi
sudo install -Dm644 packaging/tplight.service "${UNIT}"
if [[ ${UDEV} -eq 1 ]]; then
    sudo groupadd --system tplight 2>/dev/null || true
    sudo usermod -aG tplight "${USER}"
    sudo install -Dm644 packaging/70-acer-touchpad-light.rules "${RULE}"
    sudo udevadm control --reload
    sudo udevadm trigger --subsystem-match=hidraw
    echo "    udev rule installed; log out and in again to use tplight without sudo"
fi
sudo "${BIN}" daemon --check
sudo systemctl daemon-reload
sudo systemctl enable --now tplight.service
sudo systemctl restart tplight.service
echo "==> Done. Logs: journalctl -u tplight -f"
