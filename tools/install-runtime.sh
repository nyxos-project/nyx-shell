#!/usr/bin/env bash
# Install a vendored Quickshell runtime (see vendor-runtime.sh) for the current user,
# unless the one installed is newer. Components' install.sh call this:
#
#   bash qml/runtime/install-runtime.sh qml/runtime
set -euo pipefail

src=${1:?usage: install-runtime.sh RUNTIME_DIR}
data=${XDG_DATA_HOME:-$HOME/.local/share}
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
dest="$data/nyx-shell/qml"

stamp_of() { sed -n 's/.*"stamp": *\([0-9]*\).*/\1/p' "$1" 2>/dev/null || echo 0; }
new=$(stamp_of "$src/runtime.json")
old=$(stamp_of "$dest/runtime.json")
if [[ -n $old && $old -gt $new ]]; then
  echo "the installed Quickshell runtime is newer; kept"
else
  tmp="$data/nyx-shell/qml.new"
  rm -rf "$tmp"
  mkdir -p "$data/nyx-shell"
  cp -r "$src/qml" "$tmp"
  cp "$src/runtime.json" "$tmp/"
  rm -rf "$dest.old"
  [[ -d $dest ]] && mv "$dest" "$dest.old"
  mv "$tmp" "$dest"
  rm -rf "$dest.old"
fi
mkdir -p "$units" "$data/nyx-shell/components"
install -m 644 "$src/systemd/nyx-shell-qs.service" "$units/nyx-shell-qs.service"
systemctl --user daemon-reload 2>/dev/null || true
