#!/bin/sh
# Build Arbor on FreeBSD (or another BSD) and package it as a tarball.
#
# Tauri's CLI isn't published for the BSDs, so this does what `tauri build`
# does by hand: the web UI must already be built into ./dist (`npm ci &&
# npm run build`, on any OS), then cargo builds the app with the UI embedded.
#
# Needs: rust, pkgconf, webkit2-gtk_41, gtk3, librsvg2-rust, libsoup3.
# Run from the repository root. Output: dist-bundles/Arbor_<version>_<os>_<arch>.tar.gz
set -eu

if [ ! -f dist/index.html ]; then
  echo "The web UI isn't built. Run 'npm ci && npm run build' first (any OS), then copy dist/ here." >&2
  exit 1
fi

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' src-tauri/Cargo.toml | head -n 1)
os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)

(cd src-tauri && cargo build --release --locked --features tauri/custom-protocol)

# Cargo's output folder: $CARGO_TARGET_DIR if set (relative paths are relative
# to src-tauri, where cargo ran), otherwise src-tauri/target.
case "${CARGO_TARGET_DIR:-}" in
  "") target=src-tauri/target ;;
  /*) target=$CARGO_TARGET_DIR ;;
  *) target=src-tauri/$CARGO_TARGET_DIR ;;
esac

name="arbor-${version}-${os}-${arch}"
stage="dist-bundles/${name}"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/icons/hicolor/128x128/apps" \
  "$stage/share/icons/hicolor/32x32/apps" "$stage/share/metainfo" "$stage/share/doc/arbor"

install -m 755 "$target/release/arbor" "$stage/bin/arbor"
install -m 644 src-tauri/icons/128x128.png "$stage/share/icons/hicolor/128x128/apps/arbor.png"
install -m 644 src-tauri/icons/32x32.png "$stage/share/icons/hicolor/32x32/apps/arbor.png"
install -m 644 LICENSE THIRD_PARTY_NOTICES.md README.md "$stage/share/doc/arbor/"
# AppStream metadata, for software centres.
install -m 644 src-tauri/linux/app.arbor.git.metainfo.xml "$stage/share/metainfo/"

# Named after Arbor's application id ("Arbor", see src-tauri/src/desktop.rs),
# so docks and task switchers match the window to this launcher and its icon.
cat > "$stage/share/applications/Arbor.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Arbor
GenericName=Git client
Comment=A friendly Git client
Exec=arbor %F
Icon=arbor
Terminal=false
Categories=Development;RevisionControl;
Keywords=git;version control;vcs;
StartupWMClass=Arbor
StartupNotify=true
EOF

cat > "$stage/install.sh" <<'EOF'
#!/bin/sh
# Install Arbor into $PREFIX (default /usr/local). Run as root, e.g. `doas ./install.sh`.
set -eu
PREFIX=${PREFIX:-/usr/local}
cd "$(dirname "$0")"
install -d "$PREFIX/bin" "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/128x128/apps" \
  "$PREFIX/share/icons/hicolor/32x32/apps" "$PREFIX/share/metainfo" "$PREFIX/share/doc/arbor"
install -m 755 bin/arbor "$PREFIX/bin/arbor"
install -m 644 share/applications/Arbor.desktop "$PREFIX/share/applications/"
# Older versions installed it in lower case.
rm -f "$PREFIX/share/applications/arbor.desktop"
install -m 644 share/icons/hicolor/128x128/apps/arbor.png "$PREFIX/share/icons/hicolor/128x128/apps/"
install -m 644 share/icons/hicolor/32x32/apps/arbor.png "$PREFIX/share/icons/hicolor/32x32/apps/"
install -m 644 share/metainfo/app.arbor.git.metainfo.xml "$PREFIX/share/metainfo/"
install -m 644 share/doc/arbor/* "$PREFIX/share/doc/arbor/"
echo "Installed to $PREFIX/bin/arbor"
EOF
chmod 755 "$stage/install.sh"

cat > "$stage/README.txt" <<EOF
Arbor ${version} for $(uname -s) ${arch}

1. Install the runtime libraries:   pkg install webkit2-gtk_41 git
2. Install Arbor (as root):         ./install.sh
   or run it in place:               ./bin/arbor

Arbor is free software under the GNU General Public License, version 3 or
later; see share/doc/arbor/LICENSE.
EOF

tar -C dist-bundles -czf "dist-bundles/Arbor_${version}_${os}_${arch}.tar.gz" "$name"
rm -rf "$stage"
ls -l dist-bundles
