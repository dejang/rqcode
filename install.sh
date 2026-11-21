#!/bin/sh
set -eu

REPO="dejang/rqcode"
BINARY="rqcode"
INSTALL_DIR="${RQCODE_INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${RQCODE_VERSION:-latest}"

say() {
    printf '%s\n' "$*"
}

err() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        err "required command not found: $1"
    fi
}

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            case "$arch" in
                arm64 | aarch64) printf '%s' "aarch64-apple-darwin" ;;
                x86_64) printf '%s' "x86_64-apple-darwin" ;;
                *) err "unsupported macOS architecture: ${arch}. Install via Cargo instead: cargo install rqcode" ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64 | amd64) printf '%s' "x86_64-unknown-linux-gnu" ;;
                aarch64 | arm64) printf '%s' "aarch64-unknown-linux-gnu" ;;
                *) err "unsupported Linux architecture: ${arch}. Install via Cargo instead: cargo install rqcode" ;;
            esac
            ;;
        *)
            err "no prebuilt binary available for ${os} (${arch}). Install via Cargo instead: cargo install rqcode"
            ;;
    esac
}

verify_sha256() {
    file="$1"
    expected="$2"

    if command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$file" | awk '{print $1}')"
    elif command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$file" | awk '{print $1}')"
    else
        err "neither shasum nor sha256sum found; cannot verify download checksum"
    fi

    if [ "$actual" != "$expected" ]; then
        err "checksum mismatch for ${file} (expected ${expected}, got ${actual})"
    fi
}

main() {
    need_cmd curl
    need_cmd tar

    target="$(detect_target)"
    archive="${BINARY}-${target}.tar.gz"

    if [ "$VERSION" = "latest" ]; then
        base_url="https://github.com/${REPO}/releases/latest/download"
    else
        base_url="https://github.com/${REPO}/releases/download/${VERSION}"
    fi

    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT INT TERM

    say "Downloading ${BINARY} (${VERSION}, ${target})..."
    curl -fsSL "${base_url}/${archive}" -o "${tmp_dir}/${archive}"
    curl -fsSL "${base_url}/${archive}.sha256" -o "${tmp_dir}/${archive}.sha256"

    expected_sha="$(awk '{print $1}' "${tmp_dir}/${archive}.sha256")"
    [ -n "$expected_sha" ] || err "empty checksum file"
    verify_sha256 "${tmp_dir}/${archive}" "$expected_sha"

    tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir"
    [ -f "${tmp_dir}/${BINARY}" ] || err "archive did not contain ${BINARY} binary"

    mkdir -p "$INSTALL_DIR"
    cp "${tmp_dir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    chmod 755 "${INSTALL_DIR}/${BINARY}"

    say "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            say ""
            say "Add ${INSTALL_DIR} to your PATH to run '${BINARY}' directly:"
            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac
}

main "$@"
