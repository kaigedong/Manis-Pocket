# Manis Pocket AUR binary package

`manis-pocket-bin` installs the native GTK4 app `/usr/bin/manis-pocket`, its
application menu entry and icon, and the Wayland sync backend at
`/usr/bin/manis-pocket-wayland`. It repackages an immutable Arch Linux package
from a GitHub release.

The runtime dependency on `wl-clipboard` supplies `wl-copy` and `wl-paste`.
Open **Manis Pocket** from the application menu in a Wayland session, or run
`manis-pocket` from a terminal. The GUI handles PIN confirmation. Closing its
window keeps synchronization running; use **Quit** to stop it.

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

Run `manis-pocket` in a Wayland session. See the
project [README](../../README.md) for pairing and clipboard instructions.
