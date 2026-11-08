# Packaging patches

Changes for the repositories that ship Nyx Shell on NyxOS, against their heads of
2026-11-07 (nyx-packages `455b60d`, nyxos `f107b41`):

- `nyx-packages.patch`: the `nyx-shell` flake input, `pkgs/nyx-shell.nix`, the
  `nyx.shell` option and its defaults layer. `flake.lock` isn't in it: lock after
  applying.
- `nyxos.patch`: `nyx.shell.enable = lib.mkDefault false` in `modules/nixos/apps.nix`,
  and the monitor hook and shortcuts file in `modules/home/hyprland.nix`. nyxos needs a
  nyx-packages with the option first (`nix flake update nyx-packages`).

How they were checked and what to run: [../docs/NYXOS.md](../docs/NYXOS.md).
