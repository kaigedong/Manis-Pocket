# Manis Pocket AUR binary package

`manis-pocket-bin` installs the Manis Pocket Wayland clipboard client at
`/usr/bin/manis-pocket-wayland`. It repackages an immutable Arch Linux package
from a GitHub release. The macOS GUI is not included.

The runtime dependency on `wl-clipboard` supplies `wl-copy` and `wl-paste`.
Pairing requires interactive PIN confirmation, so the package does not install
a desktop entry or a system service.

## Release flow

The [Arch CI](../../.github/workflows/arch-package.yml) builds and tests the
checkout on every push to `master`, then uploads a package and its SHA-256 as
workflow artifacts. Once the run succeeds, dispatch
[Publish Arch release and AUR](../../.github/workflows/publish-aur.yml) with
that Arch CI run ID. The publisher checks the run's commit, creates an immutable
`wayland-${pkgver}` GitHub release, renders this package from `PKGBUILD.in`,
generates `.SRCINFO` on Arch, and pushes both files to AUR.

The publisher needs `AUR_SSH_PRIVATE_KEY` in the **Manis-Pocket** GitHub
repository. Its public key must be registered with the `bobosingle` AUR account.

## Local validation

After a GitHub release exists, render and build the AUR package on Arch Linux:

```sh
bash packaging/aur/render-pkgbuild.sh PKGVER RELEASE_SHA256 /path/to/aur-checkout
cd /path/to/aur-checkout
makepkg --syncdeps --cleanbuild
makepkg --printsrcinfo > .SRCINFO
namcap manis-pocket-bin-*.pkg.tar.zst
```

Run `manis-pocket-wayland --name "Linux Laptop"` in a Wayland session. See the
project [README](../../README.md) for pairing and clipboard instructions.
