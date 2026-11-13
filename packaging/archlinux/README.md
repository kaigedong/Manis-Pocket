# Arch Linux package from this checkout

This PKGBUILD builds Manis Pocket for Linux from the current repository
checkout. It installs the GPUI desktop app `/usr/bin/manis-pocket`, a desktop
entry and icon, and the MIT license. `wl-clipboard` and the
Wayland/XKB/Fontconfig runtime libraries are dependencies. The GUI handles
interactive PIN confirmation.

On Arch Linux x86_64 with `base-devel` installed:

```sh
cd packaging/archlinux
makepkg --syncdeps --cleanbuild
namcap manis-pocket-[0-9]*.pkg.tar.zst
sudo pacman -U manis-pocket-[0-9]*.pkg.tar.zst
```

Sync runs inside the application, so the package ships a single binary.

For the AUR binary package, see [../aur](../aur/README.md). It repackages the
package attached to the product release by the release workflow. This
checkout package is intended for CI artifacts and local validation.
