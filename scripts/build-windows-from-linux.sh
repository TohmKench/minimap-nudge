#!/usr/bin/env bash
# Cross-compiles the Windows exe + NSIS installer on Ubuntu 24.04 (e.g. asgard).
# On Windows itself just run `npm run build` instead.
#
# One-time setup:
#   sudo apt install rustc-1.97 cargo-1.97 rust-1.97-src mingw-w64 nsis
#   ./scripts/build-windows-from-linux.sh --setup
# Ubuntu's rust-src strips std's Windows dependency, so --setup makes a patched
# copy and std is rebuilt for Windows with -Zbuild-std (needs RUSTC_BOOTSTRAP).
set -euo pipefail
cd "$(dirname "$0")/.."
RUST=/usr/lib/rust-1.97
SRC=${MN_STD_SRC:-$HOME/.cache/minimap-nudge/rust-std-src}

if [[ "${1:-}" == "--setup" ]]; then
  rm -rf "$SRC" && mkdir -p "$SRC"
  cp -a /usr/src/rustc-1.97.*/library "$SRC/library"
  python3 - "$SRC/library/std/Cargo.toml" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
if "windows-link" not in s.split("[features]")[0]:
    s = s.replace("""[target.'cfg(not(all(windows, target_env = "msvc")))'.dependencies]""",
      """[target.'cfg(windows)'.dependencies.windows-link]\npath = "../windows_link"\n\n[target.'cfg(not(all(windows, target_env = "msvc")))'.dependencies]""", 1)
    s = s.replace("windows_raw_dylib = []", 'windows_raw_dylib = ["windows-link/windows_raw_dylib"]')
open(p, "w").write(s)
PY
  echo "std source prepared in $SRC"
  exit 0
fi

# rustup (if present) would reject the target; this build uses Ubuntu's toolchain only
PATH="$(echo "$PATH" | tr : "\n" | grep -v "\.cargo/bin" | paste -sd:)"
export PATH="$RUST/bin:$PATH" RUSTC_BOOTSTRAP=1 __CARGO_TESTS_ONLY_SRC_ROOT="$SRC/library"
npx tauri build --target x86_64-pc-windows-gnu --bundles nsis -- -Zbuild-std
echo
ls -la src-tauri/target/x86_64-pc-windows-gnu/release/*.exe src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/*.exe
