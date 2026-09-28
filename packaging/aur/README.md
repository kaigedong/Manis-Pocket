# Arch Linux / AUR

This source package installs the **Manis Pocket Wayland CLI** as
`/usr/bin/manis-pocket-wayland`. It does not include the macOS GUI. The package
uses the renamed `Manis-Pocket` repository and binary; it is separate from the
unrelated `Manis` proxy app and its `manis-bin` AUR package.

Unlike Manis's `manis-bin` package, there is no Linux release artifact to
repackage yet. `manis-pocket-wayland-git` builds the Rust binary from the
upstream `master` branch. The package depends on `wl-clipboard` for `wl-copy`
and `wl-paste`, and installs no desktop entry or system service because pairing
requires interactive PIN confirmation.

## Local Arch build

Before pushing the source revision, validate the current checkout with
[the checkout PKGBUILD](../archlinux/README.md). After pushing, test this AUR
PKGBUILD on an Arch Linux x86_64 Wayland machine with `base-devel` installed:

```sh
cd packaging/aur
makepkg --syncdeps --cleanbuild
namcap manis-pocket-wayland-git-*.pkg.tar.zst
```

Install the resulting package with `sudo pacman -U` and run
`manis-pocket-wayland --name "Linux Laptop"` in the Wayland session. See the
project [README](../../README.md) for pairing and clipboard instructions.

## AUR publication

The source revision containing the Wayland client must be pushed to
`kaigedong/Manis-Pocket` before publishing this PKGBUILD. Then copy `PKGBUILD`
to an AUR checkout of `ssh://aur@aur.archlinux.org/manis-pocket-wayland-git.git`.
Build in that checkout so `pkgver()` updates the version from the fetched
commit, then generate `.SRCINFO` from the updated PKGBUILD:

```sh
makepkg --syncdeps --cleanbuild
makepkg --printsrcinfo > .SRCINFO
namcap manis-pocket-wayland-git-*.pkg.tar.zst
git add PKGBUILD .SRCINFO
git commit -m 'Publish Manis Pocket Wayland client'
git push origin HEAD:master
```

Only push after a clean Arch build and a Wayland session clipboard smoke test.
The AUR checkout should contain `PKGBUILD` and `.SRCINFO`; it must not contain
the generated package archive.
