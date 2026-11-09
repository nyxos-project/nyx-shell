#!/usr/bin/env bash
# Copy the shared Quickshell runtime into a component's repository (qml/runtime), so the
# component installs it on desktops without Nyx Shell (Omarchy):
#
#   tools/vendor-runtime.sh ~/Projects/nyx-dock/qml/runtime
#
# The runtime: shell.qml (loads components), the Nyx.Shell, Nyx.Theme, Nyx.Ui and
# Nyx.PluginView modules, and the standalone nyx-shell-qs.service. runtime.json records
# the revisions it came from and their commit time; install.sh only replaces an
# installed runtime with a newer one, so components vendoring different revisions never
# go back in time.
set -euo pipefail

dest=${1:?usage: vendor-runtime.sh DEST}
shell_repo=$(cd "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
design_repo=${NYX_DESIGN:-$shell_repo/../nyx-design}
[[ -d $design_repo/qml/Nyx/Theme ]] || { echo "nyx-design not found at $design_repo (set NYX_DESIGN)" >&2; exit 1; }

rev() { git -C "$1" rev-parse HEAD; }
stamp() { git -C "$1" log -1 --format=%ct; }
dirty() { [[ -n $(git -C "$1" status --porcelain -- qml) ]] && echo true || echo false; }

rm -rf "$dest"
mkdir -p "$dest/qml/Nyx" "$dest/systemd"
cp "$shell_repo/qml/shell.qml" "$dest/qml/"
cp -r "$shell_repo/qml/examples" "$dest/qml/"
cp -r "$shell_repo/qml/Nyx/Shell" "$dest/qml/Nyx/"
for m in Theme Ui PluginView; do
  cp -r "$design_repo/qml/Nyx/$m" "$dest/qml/Nyx/"
done
cp "$shell_repo/data/standalone/nyx-shell-qs.service" "$dest/systemd/"
cp "$shell_repo/tools/install-runtime.sh" "$dest/"
s1=$(stamp "$shell_repo"); s2=$(stamp "$design_repo")
cat > "$dest/runtime.json" <<EOF
{
  "nyx-shell": "$(rev "$shell_repo")",
  "nyx-shell-dirty": $(dirty "$shell_repo"),
  "nyx-design": "$(rev "$design_repo")",
  "nyx-design-dirty": $(dirty "$design_repo"),
  "stamp": $(( s1 > s2 ? s1 : s2 ))
}
EOF
echo "runtime copied to $dest"
