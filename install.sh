#!/usr/bin/env bash
# Build Utilify from this checkout and install it into /Applications.
#
#   ./install.sh
#
# Environment:
#   UTILIFY_INSTALL_DIR  where to put Utilify.app (default /Applications,
#                         or ~/Applications if that isn't writable)
#   UTILIFY_NO_LAUNCH=1  don't open the app afterwards
#   UTILIFY_NO_FAN_HELPER=1  don't install the fan-control helper (and so
#                         don't ask for sudo); fans stay read-only
#   UTILIFY_CLI_DIR      where to link the utilifyctl command (default: the
#                         first writable of /opt/homebrew/bin, /usr/local/bin,
#                         ~/.local/bin)
#   UTILIFY_NO_CLI=1     don't link utilifyctl
set -euo pipefail

APP="Utilify.app"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == Darwin ]] || die "Utilify only runs on macOS."
command -v cargo >/dev/null || die "cargo not found. Install Rust from https://rustup.rs and re-run."

# Build the checkout this script lives in, whatever the current directory.
cd "$(dirname "${BASH_SOURCE[0]}")"
[[ -f crates/utilify-app/Cargo.toml ]] || die "run this from the Utilify source tree"

if ! command -v cargo-packager >/dev/null; then
  say "Installing cargo-packager"
  cargo install cargo-packager --locked
fi

say "Building $APP (release; the first build takes a few minutes)"
cargo packager --release -p utilify-app --formats app
built="target/packages/$APP"
[[ -d "$built" ]] || die "build finished but $built is missing"

dest="${UTILIFY_INSTALL_DIR:-/Applications}"
if [[ -z "${UTILIFY_INSTALL_DIR:-}" && ! -w "$dest" ]]; then
  dest="$HOME/Applications"
fi
mkdir -p "$dest"
# Absolute, so the utilifyctl symlink below points at the right place.
dest="$(cd "$dest" && pwd)"

# Replace a running copy.
if pgrep -xq utilify; then
  say "Quitting the running Utilify"
  osascript -e 'quit app "Utilify"' >/dev/null 2>&1 || pkill -x utilify || true
  sleep 1
fi

say "Installing to $dest/$APP"
rm -rf "${dest:?}/$APP"
cp -R "$built" "$dest/"
# Clear quarantine in case the source tree was downloaded rather than cloned.
xattr -dr com.apple.quarantine "$dest/$APP" 2>/dev/null || true

# The command line: utilifyctl lives inside the bundle, so a symlink keeps
# it in step with the app on every install.
cli="$dest/$APP/Contents/MacOS/utilifyctl"
if [[ "${UTILIFY_NO_CLI:-}" == 1 ]]; then
  say "Skipping the utilifyctl link (UTILIFY_NO_CLI=1)"
elif [[ -x "$cli" ]]; then
  bin_dir="${UTILIFY_CLI_DIR:-}"
  if [[ -z "$bin_dir" ]]; then
    for d in /opt/homebrew/bin /usr/local/bin "$HOME/.local/bin"; do
      if [[ -d "$d" && -w "$d" ]]; then bin_dir="$d"; break; fi
    done
    bin_dir="${bin_dir:-$HOME/.local/bin}"
  fi
  mkdir -p "$bin_dir"
  ln -sf "$cli" "$bin_dir/utilifyctl"
  say "Linked utilifyctl into $bin_dir"
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "    $bin_dir isn't on your PATH; add it to run utilifyctl by name." ;;
  esac
else
  printf '\033[33mwarning:\033[0m %s is missing; utilifyctl not linked\n' "$cli" >&2
fi

# Fan control. Setting fan speeds means writing to the SMC, which macOS only
# allows root, so a small helper runs as a root LaunchDaemon and takes
# requests from Utilify over a socket only admin users can open. Reading
# fans and temperatures works without it.
HELPER_LABEL="dev.utilify.fanhelper"
HELPER_BIN="/Library/PrivilegedHelperTools/$HELPER_LABEL"
HELPER_PLIST="/Library/LaunchDaemons/$HELPER_LABEL.plist"
if [[ "${UTILIFY_NO_FAN_HELPER:-}" == 1 ]]; then
  say "Skipping the fan helper (UTILIFY_NO_FAN_HELPER=1)"
elif [[ "$(sysctl -n hw.optional.arm64 2>/dev/null)" != 1 ]]; then
  say "Skipping the fan helper: fan control needs an Apple Silicon Mac"
else
  say "Building the fan helper (release)"
  cargo build --release -p utilify-fanhelper
  helper="target/release/utilify-fanhelper"
  [[ -x "$helper" ]] || die "build finished but $helper is missing"

  plist="$(mktemp -t utilify-fanhelper)"
  cat >"$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$HELPER_LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$HELPER_BIN</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardErrorPath</key>
  <string>/var/log/utilify-fanhelper.log</string>
</dict>
</plist>
PLIST

  say "Installing the fan helper as a root LaunchDaemon ($HELPER_LABEL)"
  echo "    Setting fan speeds writes to the Mac's SMC, which only root may do, so"
  echo "    Utilify installs a small helper that runs as root and only changes fan"
  echo "    modes and target speeds, for admin users, returning the fans to"
  echo "    automatic when Utilify lets go. sudo asks for your password once."
  echo "    (Skip this with UTILIFY_NO_FAN_HELPER=1; remove it with"
  echo "    scripts/uninstall-fan-helper.sh.)"
  # One sudo for everything. bootout stops a running helper (which puts the
  # fans back to automatic) so the new binary loads; bootstrap can briefly
  # fail while the old job is still going away, so it retries.
  if sudo /bin/bash -euc '
      mkdir -p /Library/PrivilegedHelperTools
      install -o root -g wheel -m 755 "$1" "$2"
      install -o root -g wheel -m 644 "$3" "$4"
      launchctl bootout "system/$5" 2>/dev/null || true
      for _ in 1 2 3 4 5; do
        launchctl bootstrap system "$4" 2>/dev/null && exit 0
        sleep 1
      done
      launchctl bootstrap system "$4"
    ' _ "$helper" "$HELPER_BIN" "$plist" "$HELPER_PLIST" "$HELPER_LABEL"; then
    say "Fan helper running (log: /var/log/utilify-fanhelper.log)"
  else
    printf '\033[33mwarning:\033[0m fan helper not installed; fan control stays read-only\n' >&2
  fi
  rm -f "$plist"
fi

if [[ "${UTILIFY_NO_LAUNCH:-}" != 1 ]]; then
  open "$dest/$APP"
  say "Utilify is running: look for it in the menu bar."
else
  say "Installed. Open $dest/$APP to start it."
fi
