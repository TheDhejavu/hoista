#!/bin/sh
# Installs a Hoista release: downloads it, checks its signature, and puts
# hoist and hoistd in ~/.local/bin (or the folder you give). By default
# Hoista then starts at login; --no-service installs the programs only.
#
#   sh install.sh v0.1.0-beta.1 [destination] [--start-at-login | --no-service]
#
# Needs cosign 3: an unsigned or altered release is never installed.
set -eu

REPOSITORY=TheDhejavu/hoista
RELEASE_FILES='hoist hoistd LICENSE THIRD-PARTY-NOTICES.txt DEPENDENCIES.json'

fail() {
  echo "$1" >&2
  exit 1
}

# Arguments
[ "$#" -le 3 ] || fail 'Too many arguments.'
version=${1:?Give the release to install, for example v0.1.0-beta.1}
destination=${2:-"$HOME/.local/bin"}
service_mode=${3:---start-at-login}

case "$version" in
  v[0-9]*) ;;
  *) fail 'Invalid release version.' ;;
esac
case "$version" in
  *[!A-Za-z0-9.-]*) fail 'Invalid release version.' ;;
esac
case "$service_mode" in
  --start-at-login | --no-service) ;;
  *) fail 'Choose --start-at-login or --no-service.' ;;
esac

command -v cosign >/dev/null 2>&1 || fail 'Install cosign 3 first; signatures are mandatory.'

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target=aarch64-apple-darwin ;;
  Darwin:x86_64) target=x86_64-apple-darwin ;;
  Linux:x86_64) target=x86_64-unknown-linux-gnu ;;
  *) fail 'Unsupported platform; build from source.' ;;
esac

# Download the release and its signature, and check the signature came from
# this repository's release workflow for this exact tag.
workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT HUP INT TERM

archive="hoista-$target.tar.gz"
base="https://github.com/$REPOSITORY/releases/download/$version"

download() {
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' "$1" -o "$2"
}

download "$base/$archive" "$workdir/$archive"
download "$base/$archive.sigstore.json" "$workdir/signature.json"

cosign verify-blob \
  --bundle "$workdir/signature.json" \
  --certificate-identity "https://github.com/$REPOSITORY/.github/workflows/release.yml@refs/tags/$version" \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  "$workdir/$archive"

# Unpack only the expected files, and only as regular files.
tar -tzf "$workdir/$archive" | while IFS= read -r entry; do
  case " $RELEASE_FILES " in
    *" $entry "*) ;;
    *) fail 'Unexpected file in the release.' ;;
  esac
done
# The file list is split into one argument per name on purpose.
# shellcheck disable=SC2086
tar -xzf "$workdir/$archive" -C "$workdir" $RELEASE_FILES
for file in $RELEASE_FILES; do
  [ -f "$workdir/$file" ] && [ ! -L "$workdir/$file" ] || fail 'Release files must be regular files.'
done

# Install.
mkdir -p "$destination" "$destination/hoista-notices"
install -m 755 "$workdir/hoist" "$destination/hoist"
install -m 755 "$workdir/hoistd" "$destination/hoistd"
for document in LICENSE THIRD-PARTY-NOTICES.txt DEPENDENCIES.json; do
  install -m 644 "$workdir/$document" "$destination/hoista-notices/$document"
done
printf 'Installed Hoista to %s. Add this directory to PATH.\n' "$destination"

if [ "$service_mode" = --start-at-login ]; then
  "$destination/hoist" service install ||
    fail 'Hoista is installed, but it could not be set to start at login. Run hoist service install from your own login session to try again, or hoist start for now.'
else
  echo 'Hoista won’t start at login. Run hoist service install when you want it to; any hoist command also starts it when needed.'
fi
echo 'Next: hoist accounts add, then hoist mcp config to connect your coding tool.'
