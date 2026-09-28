# Arch Linux package from this checkout

This PKGBUILD builds Manis Pocket for Linux from the current repository
checkout. It installs the GTK4 app `/usr/bin/manis-pocket`, a desktop
entry and icon, the sync backend `/usr/bin/manis-pocket-wayland`, and the MIT
license. GTK4 and `wl-clipboard` are runtime dependencies. The GUI handles
interactive PIN confirmation.

On Arch Linux x86_64 with `base-devel` installed:

```sh
cd packaging/archlinux
makepkg --syncdeps --cleanbuild
namcap manis-pocket-wayland-[0-9]*.pkg.tar.zst
sudo pacman -U manis-pocket-wayland-[0-9]*.pkg.tar.zst
```

For the AUR binary package, see [../aur](../aur/README.md). It repackages the
immutable Arch package published by the release workflow. This checkout
package is intended for CI artifacts and local validation.
