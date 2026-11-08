# Original UFLinux artwork

This desktop restores the icon theme and default wallpaper selected by the
[original UFLinux configuration](https://github.com/UFlinux-project/main/blob/067f3069b20f78995920d30795d48d36c96fcdf7/config/includes.chroot_before_packages/etc/dconf/db/local.d/00_uflinux_defaults):
Papirus and GNOME's `Dark_Ivy.jpg`. Those assets were third-party artwork used by
the original distribution, rather than artwork authored by UFLinux.

Papirus comes from Alpine's signed `papirus-icon-theme` package. GTK3, GTK4,
labwc and Fuzzel select Papirus by default; Ironbar uses the GTK theme for its
application icons. Adwaita remains available for fallback icons. Personal XDG
configuration continues to override the packaged defaults.

GTK3 and GTK4 apps use `adw-gtk3-dark` from Alpine's signed `adw-gtk3` package.
Both toolkit versions receive the theme name and dark preference through their
XDG `settings.ini` files. This is a CSS theme; UFLinux Settings remains a plain
GTK4 app without a libadwaita dependency. No global `GTK_THEME` override is set,
so personal themes and accessibility preferences can take precedence. The theme
package version and hashes are recorded with the image's resolved APK inventory.

Six original GNOME 3.38 wallpapers are installed without image modifications in
`/usr/share/backgrounds/uflinux/`: Dark Ivy, Cold Warm, Endless Shapes, Road and
the two Symbolics images. This subset has explicit upstream attribution and fits
the Settings app's 16-megapixel image limit. Dark Ivy is the login default;
Appearance's wallpaper chooser starts in the bundled collection. Restore default
wallpaper also selects Dark Ivy. No GNOME desktop or settings daemon is needed.

The leaf logo is copied unchanged from the original
[Calamares branding](https://github.com/UFlinux-project/uf-calamares-settings/blob/53217a0b7edc494658a1dfcfee649365a19d0c79/etc/calamares/branding/uflinux/uflinuxlogo-spaces.svg).
It appears on Ironbar's Start button, the Settings application icon and About.
The imported SVGs are checked against the exact upstream bytes recorded in
`config/artwork.lock.json`.

The GNOME 3.38.0 source archive URL and SHA-512 are pinned in
`config/sources.lock.json` and APKBUILD. Its SHA-256 was also checked against
GNOME's published checksum file. The package builder retains the archive with
other rebuild inputs, and APKBUILD verifies it during extraction. No wallpaper
is fetched at login. Papirus versions and APK hashes are recorded in each image's
normal resolved-package inventory.

The logo retains GPL-3.0-only. Dark Ivy is by Garrett LeSage; Endless Shapes and
the Symbolics images are by Jakub Steiner, under CC BY-SA 3.0. Cold Warm is by
invisiblepower and Road by Joshua Welch, under CC0. The exact GNOME attribution
and license notice, GPL text and artwork notice are installed in
`/usr/share/licenses/uflinux-defaults/`. Settings' copy of the logo is covered by
its GPL license and source attribution in `settings/README.md`.

The package/desktop verification suite checks Papirus and branded icon lookup,
SVG rendering, every bundled image's dimensions, bounded previews, the Start
configuration and actual wallpaper process startup. Historical ISO checksums and
measurements in earlier validation documents describe their original artifacts;
these defaults take effect in subsequent builds.

Local validation on 8 November 2026 passed rebuilt UFLinux/Settings packages,
fresh signed installation of the 21-package closure, the artwork rendering checks
and the complete headless desktop suite. Papirus availability checks passed for
all eight architectures; 38 Python tests, Rust package tests, ShellCheck and
workflow linting also passed. Reports and screenshots are in
`.cache/artwork-out/checks/x86_64/`. The other architectures and a new ISO were
not built during this artwork change.

The adw-gtk3 follow-up also passed independent GTK3/GTK4 theme probes and the
full native desktop suite. Reports are in `.cache/adw-out/checks/x86_64/`.
