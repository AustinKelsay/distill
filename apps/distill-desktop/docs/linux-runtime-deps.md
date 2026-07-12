# Linux package and smoke prerequisites

The supported CI/release baseline for issue #36 is Ubuntu 24.04 x86_64. The Linux
package command emits both a Debian package and an AppImage:

```bash
npm run desktop:package:linux
```

The installed-host smoke uses the Debian package as the primary install proof. The
AppImage is built and uploaded as a portable artifact; runners without FUSE should use
`APPIMAGE_EXTRACT_AND_RUN=1` for any optional launch check.

Build dependencies are `build-essential`, `pkg-config`, `libgtk-3-dev`,
`libwebkit2gtk-4.1-dev`, `librsvg2-dev`, and `patchelf`. Runtime package metadata
declares WebKitGTK 4.1 and GTK 3 (`libwebkit2gtk-4.1-0` and
`libgtk-3-0 | libgtk-3-0t64`). The smoke runner additionally installs `xvfb`,
`dbus-x11`, `at-spi2-core`, `gir1.2-atspi-2.0`, `python3-gi`, and `xdotool`.

The CI workflow is
`.github/workflows/linux-package-smoke.yml`. It builds the real Tauri packages,
installs the `.deb` with `apt-get`, verifies its `Depends` metadata, launches
`/usr/bin/distill-desktop` under an Xvfb/dbus session, locates controls through the
AT-SPI accessible tree, and checks the Fixture-to-train-export journey, restart
persistence, capability source, Fixture hashes, and chosen-home containment.

This is package/install/runtime evidence only. It does not claim migration, crash
recovery, privacy, scale, export atomicity, or screen-reader support. The renderer
and human accessibility contracts remain separate gates.
