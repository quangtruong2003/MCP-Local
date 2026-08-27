#!/usr/bin/env bash
# Builds the immutable Ubuntu 24.04 (noble) minbase rootfs used by the managed
# WSL2 Lightpanda runtime. Designed for ubuntu-24.04 CI runners; also works
# inside an existing WSL Ubuntu distro with root/sudo.
#
# The artifact is a deterministic tar.gz suitable for `wsl.exe --import`.
# Lightpanda itself is NOT baked into the rootfs; the app provisions the pinned
# upstream binary separately (this script only smoke-verifies that the rootfs
# can execute it, then removes it).
#
# Outputs (in OUT_DIR):
#   lightpanda-rootfs-ubuntu-noble-minbase-amd64.tar.gz
#   lightpanda-rootfs-ubuntu-noble-minbase-amd64.tar.gz.sha256
#   lightpanda-rootfs-packages.txt
#   lightpanda-rootfs-sbom.spdx.json
set -euo pipefail

SUITE="${SUITE:-noble}"
MIRROR="${MIRROR:-http://archive.ubuntu.com/ubuntu}"
DISTRO_ARCH="amd64"
OUT_DIR="${OUT_DIR:-$PWD/dist/lightpanda-rootfs}"
WORK_DIR="${WORK_DIR:-$(mktemp -d /tmp/lwsl-rootfs-build.XXXXXX)}"
ROOTFS="$WORK_DIR/rootfs"
# Fixed timestamp for reproducible archives (2026-08-27T00:00:00Z).
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1787990400}"

LIGHTPANDA_VERSION="0.3.7"
LIGHTPANDA_ARCH="x86_64-linux"
LIGHTPANDA_URL="https://github.com/lightpanda-io/browser/releases/download/${LIGHTPANDA_VERSION}/lightpanda-${LIGHTPANDA_ARCH}"
LIGHTPANDA_SHA256="895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a"

ARTIFACT_NAME="lightpanda-rootfs-ubuntu-noble-minbase-${DISTRO_ARCH}.tar.gz"

log() { printf '[build-rootfs] %s\n' "$*"; }
die() { printf '[build-rootfs] ERROR: %s\n' "$*" >&2; exit 1; }

# --- 0. Privileges -----------------------------------------------------------
if [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null 2>&1; then
    log "re-executing as root via sudo"
    exec sudo -E env OUT_DIR="$OUT_DIR" WORK_DIR="$WORK_DIR" SUITE="$SUITE" MIRROR="$MIRROR" \
      SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" bash "$0"
  fi
  die "must run as root (debootstrap requirement); sudo not found"
fi

command -v debootstrap >/dev/null 2>&1 || {
  log "installing debootstrap"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq debootstrap ca-certificates curl tar gzip >/dev/null
}

mkdir -p "$OUT_DIR"

# --- 1. debootstrap minbase --------------------------------------------------
if [ ! -d "$ROOTFS" ]; then
  log "debootstrap --variant=minbase --arch=$DISTRO_ARCH $SUITE from $MIRROR"
  debootstrap --variant=minbase --arch="$DISTRO_ARCH" \
    --include=ca-certificates "$SUITE" "$ROOTFS" "$MIRROR"
fi

# --- 2. Runtime user + owned directories -------------------------------------
log "creating localworkspaces user and runtime directories"
chroot "$ROOTFS" /bin/sh -c '
  set -e
  if ! id localworkspaces >/dev/null 2>&1; then
    useradd --user-group --home-dir /var/lib/local-workspaces \
      --shell /bin/sh localworkspaces
    passwd -l localworkspaces >/dev/null
  fi
  mkdir -p \
    /var/lib/local-workspaces/browser/profile \
    /var/lib/local-workspaces/browser/cookies \
    /var/lib/local-workspaces/browser/cache \
    /var/lib/local-workspaces/browser/artifacts \
    /opt/local-workspaces/lightpanda
  chown -R localworkspaces:localworkspaces /var/lib/local-workspaces
  chmod 750 /var/lib/local-workspaces
  # Executable install location stays root-owned; the app provisions binaries.
  chown -R root:root /opt/local-workspaces
'

# --- 3. Runtime library sufficiency (DNS/TLS already via ca-certificates) ----
log "verifying base runtime libraries are present"
chroot "$ROOTFS" /bin/sh -c '
  set -e
  for pkg in libc6 libgcc-s1 libstdc++6 ca-certificates; do
    dpkg -s "$pkg" >/dev/null 2>&1 || { echo "missing required package: $pkg" >&2; exit 1; }
  done
'

# --- 4. Forbidden content assertions ------------------------------------------
log "asserting Node/npm/Playwright/Chromium/Firefox/toolchains are absent"
chroot "$ROOTFS" /bin/sh -c '
  set -e
  for cmd in node npm npx playwright chromium chromium-browser google-chrome firefox gcc make; do
    if command -v "$cmd" >/dev/null 2>&1; then
      echo "forbidden binary present in rootfs: $cmd" >&2
      exit 1
    fi
  done
  if dpkg -l 2>/dev/null | grep -Ei "nodejs|npm|playwright|chromium|firefox|build-essential" >/dev/null; then
    echo "forbidden package present in rootfs" >&2
    exit 1
  fi
'

# --- 5. Lightpanda smoke verification (binary NOT kept in the rootfs) --------
log "downloading pinned lightpanda $LIGHTPANDA_VERSION for smoke verification"
LP_TMP="$WORK_DIR/lightpanda-smoke"
curl -fsSL --retry 3 -o "$LP_TMP" "$LIGHTPANDA_URL"
echo "$LIGHTPANDA_SHA256  $LP_TMP" | sha256sum -c - >/dev/null \
  || die "lightpanda smoke binary SHA-256 mismatch"
chmod +x "$LP_TMP"
cp "$LP_TMP" "$ROOTFS/tmp/lightpanda-smoke"
log "smoke-launching lightpanda inside the rootfs (ldd + exec)"
chroot "$ROOTFS" /bin/sh -c '
  set -e
  # ldd may report "not a dynamic executable" for static builds; that is fine.
  ldd /tmp/lightpanda-smoke 2>&1 | grep -i "not found" && exit 1 || true
  /tmp/lightpanda-smoke version >/dev/null 2>&1 || /tmp/lightpanda-smoke --help >/dev/null
'
rm -f "$ROOTFS/tmp/lightpanda-smoke" "$LP_TMP"
log "smoke verification passed; lightpanda removed from rootfs"

# --- 6. Slim down + package manifest + SPDX SBOM ------------------------------
log "cleaning caches and emitting package manifest"
chroot "$ROOTFS" /bin/sh -c '
  set -e
  apt-get clean >/dev/null 2>&1 || true
  rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*
  rm -rf /usr/share/doc/* /usr/share/man/* /usr/share/info/*
'
PACKAGES_OUT="$OUT_DIR/lightpanda-rootfs-packages.txt"
chroot "$ROOTFS" dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' \
  | sort > "$PACKAGES_OUT"
[ -s "$PACKAGES_OUT" ] || die "package manifest is empty"

log "emitting SPDX SBOM"
SBOM_OUT="$OUT_DIR/lightpanda-rootfs-sbom.spdx.json"
CREATED="$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -r "$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ)"
{
  printf '{\n'
  printf '  "spdxVersion": "SPDX-2.3",\n'
  printf '  "dataLicense": "CC0-1.0",\n'
  printf '  "SPDXID": "SPDXRef-DOCUMENT",\n'
  printf '  "name": "lightpanda-rootfs-ubuntu-noble-minbase-%s",\n' "$DISTRO_ARCH"
  printf '  "documentNamespace": "urn:lwsl:rootfs:%s:%s",\n' "$DISTRO_ARCH" "$SOURCE_DATE_EPOCH"
  printf '  "creationInfo": {\n'
  printf '    "created": "%s",\n' "$CREATED"
  printf '    "creators": ["Tool: build-lightpanda-rootfs.sh"]\n'
  printf '  },\n'
  printf '  "packages": [\n'
  first=1
  while IFS=$'\t' read -r pkg ver arch; do
    [ "$first" -eq 1 ] || printf ',\n'
    first=0
    printf '    {\n'
    printf '      "SPDXID": "SPDXRef-pkg-%s",\n' "$pkg"
    printf '      "name": "%s",\n' "$pkg"
    printf '      "versionInfo": "%s",\n' "$ver"
    printf '      "downloadLocation": "NOASSERTION",\n'
    printf '      "filesAnalyzed": false,\n'
    printf '      "externalRefs": [{\n'
    printf '        "referenceCategory": "PACKAGE-MANAGER",\n'
    printf '        "referenceType": "purl",\n'
    printf '        "referenceLocator": "pkg:deb/ubuntu/%s@%s?arch=%s&distro=ubuntu-24.04"\n' "$pkg" "$ver" "$arch"
    printf '      }]\n'
    printf '    }'
  done < "$PACKAGES_OUT"
  printf '\n  ]\n}\n'
} > "$SBOM_OUT"

# --- 7. Deterministic tar.gz ---------------------------------------------------
log "creating deterministic $ARTIFACT_NAME"
TAR_OUT="$OUT_DIR/$ARTIFACT_NAME"
tar --sort=name --owner=0 --group=0 --numeric-owner \
  --mtime="@$SOURCE_DATE_EPOCH" \
  --format=gnu -C "$ROOTFS" -cf - . | gzip -9 -n > "$TAR_OUT"

sha256sum "$TAR_OUT" | awk '{print $1}' > "$TAR_OUT.sha256"
log "SHA-256: $(cat "$TAR_OUT.sha256")"
log "size: $(du -h "$TAR_OUT" | cut -f1)"
log "done: $OUT_DIR"
