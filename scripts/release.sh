#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: scripts/release.sh <version> [options]

Bumps Cargo.toml & Cargo.lock, runs the full quality gate, commits, verifies
clean-tree packaging, tags and atomically pushes to the remote (triggering
.github/workflows/release.yml), and publishes to crates.io.

Arguments:
  <version>              Stable SemVer version to release (e.g. 0.2.0 or v0.2.0)

Options:
  --dry-run              Run version bump and all verification checks, then
                         revert Cargo.toml/Cargo.lock without committing,
                         pushing, or publishing
  --skip-crates          Skip `cargo publish` after pushing the commit and tag
  --remote <name>        Git remote to push to (default: $RELEASE_REMOTE or origin)
  --branch <name>        Git branch to release from (default: $RELEASE_BRANCH or main)
  -h, --help             Show this help message
EOF
}

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

read_package_field() {
    field="$1"
    awk -v field="$field" '
        /^\[package\][[:space:]]*$/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && $0 ~ ("^[[:space:]]*" field "[[:space:]]*=") {
            line = $0
            sub(/^[^"]*"/, "", line)
            sub(/".*$/, "", line)
            print line
            exit
        }
    ' Cargo.toml
}

set_package_version() {
    new_ver="$1"
    TMP_FILE="$(mktemp "${TMPDIR:-/tmp}/rqcode-cargo-toml.XXXXXX")"
    if ! awk -v ver="$new_ver" '
        BEGIN { in_pkg = 0; replaced = 0 }
        /^\[package\][[:space:]]*$/ { in_pkg = 1; print; next }
        /^\[/ { in_pkg = 0; print; next }
        in_pkg && !replaced && /^[[:space:]]*version[[:space:]]*=/ {
            printf "version = \"%s\"\n", ver
            replaced = 1
            next
        }
        { print }
        END {
            if (!replaced) {
                exit 1
            }
        }
    ' Cargo.toml > "$TMP_FILE"; then
        rm -f "$TMP_FILE"
        TMP_FILE=""
        err "failed to update [package] version in Cargo.toml"
    fi
    cat "$TMP_FILE" > Cargo.toml
    rm -f "$TMP_FILE"
    TMP_FILE=""
}

check_no_untracked_packaged_files() {
    pkg_list="$(cargo package --list)"
    untracked_list="$(git ls-files --others --exclude-standard)"
    if [ -z "$untracked_list" ]; then
        return 0
    fi
    PKG_TMP="$(mktemp "${TMPDIR:-/tmp}/rqcode-pkg-list.XXXXXX")"
    UNTRACKED_TMP="$(mktemp "${TMPDIR:-/tmp}/rqcode-untracked.XXXXXX")"
    printf '%s\n' "$pkg_list" > "$PKG_TMP"
    printf '%s\n' "$untracked_list" > "$UNTRACKED_TMP"
    conflict="$(awk 'NR == FNR { pkg[$0] = 1; next } ($0 in pkg) { print }' "$PKG_TMP" "$UNTRACKED_TMP")"
    rm -f "$PKG_TMP" "$UNTRACKED_TMP"
    PKG_TMP=""
    UNTRACKED_TMP=""
    if [ -n "$conflict" ]; then
        printf 'error: packaged path(s) contain untracked files:\n%s\n' "$conflict" >&2
        exit 1
    fi
}

main() {
    RAW_VERSION=""
    DRY_RUN=0
    SKIP_CRATES=0
    REMOTE="${RELEASE_REMOTE:-origin}"
    BRANCH="${RELEASE_BRANCH:-main}"

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --dry-run) DRY_RUN=1 ;;
            --skip-crates) SKIP_CRATES=1 ;;
            --remote)
                [ "$#" -ge 2 ] || err "--remote requires a value"
                shift
                REMOTE="$1"
                ;;
            --branch)
                [ "$#" -ge 2 ] || err "--branch requires a value"
                shift
                BRANCH="$1"
                ;;
            -h | --help)
                usage
                exit 0
                ;;
            -*)
                err "unknown option: $1 (use --help for usage)"
                ;;
            *)
                if [ -n "$RAW_VERSION" ]; then
                    err "unexpected extra argument: $1"
                fi
                RAW_VERSION="$1"
                ;;
        esac
        shift
    done

    if [ -z "$RAW_VERSION" ]; then
        usage >&2
        exit 1
    fi

    case "$RAW_VERSION" in
        *"
"*)
            err "version must not contain newlines"
            ;;
    esac

    VERSION="${RAW_VERSION#v}"
    TAG="v${VERSION}"

    if ! printf '%s\n' "$VERSION" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
        err "invalid stable SemVer version: ${RAW_VERSION} (expected X.Y.Z or vX.Y.Z; prereleases are not supported by release.yml)"
    fi

    need_cmd git
    need_cmd cargo
    need_cmd awk

    REPO_ROOT="$(git rev-parse --show-toplevel)"
    cd "$REPO_ROOT"

    TMP_FILE=""
    PKG_TMP=""
    UNTRACKED_TMP=""
    MODIFIED_MANIFESTS=0
    UNPUSHED_RELEASE=0

    cleanup() {
        if [ -n "$TMP_FILE" ]; then
            rm -f "$TMP_FILE"
        fi
        if [ -n "$PKG_TMP" ]; then
            rm -f "$PKG_TMP"
        fi
        if [ -n "$UNTRACKED_TMP" ]; then
            rm -f "$UNTRACKED_TMP"
        fi
        if [ "$MODIFIED_MANIFESTS" -eq 1 ]; then
            say "Restoring Cargo.toml and Cargo.lock..."
            git checkout HEAD -- Cargo.toml Cargo.lock
        elif [ "$UNPUSHED_RELEASE" -eq 1 ]; then
            say "Rolling back unpushed release commit and tag ${TAG}..."
            git tag -d "$TAG" >/dev/null 2>&1 || true
            git reset --keep HEAD~1
        fi
    }
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM

    CURRENT_BRANCH="$(git symbolic-ref --short HEAD 2>/dev/null || true)"
    if [ "$CURRENT_BRANCH" != "$BRANCH" ]; then
        err "releases must be cut from '${BRANCH}' (current branch: ${CURRENT_BRANCH:-detached HEAD})"
    fi

    TRACKED_DIRTY="$(git status --porcelain --untracked-files=no)"
    if [ -n "$TRACKED_DIRTY" ]; then
        err "working tree has uncommitted changes to tracked files; commit or stash first"
    fi

    check_no_untracked_packaged_files

    say "Fetching ${REMOTE} refs and tags..."
    git fetch "$REMOTE" "$BRANCH" --tags --quiet

    BEHIND="$(git rev-list --count "HEAD..${REMOTE}/${BRANCH}")"
    if [ "$BEHIND" -gt 0 ]; then
        err "local '${BRANCH}' is ${BEHIND} commit(s) behind ${REMOTE}/${BRANCH}; pull or rebase first"
    fi

    if git rev-parse --verify --quiet "refs/tags/${TAG}" >/dev/null 2>&1; then
        err "tag ${TAG} already exists locally"
    fi

    REMOTE_TAG="$(git ls-remote --tags "$REMOTE" "refs/tags/${TAG}")"
    if [ -n "$REMOTE_TAG" ]; then
        err "tag ${TAG} already exists on ${REMOTE}"
    fi

    PACKAGE_NAME="$(read_package_field name)"
    REPO_URL="$(read_package_field repository)"
    CURRENT_VERSION="$(read_package_field version)"
    if [ -z "$PACKAGE_NAME" ] || [ -z "$CURRENT_VERSION" ]; then
        err "could not read [package] name/version from Cargo.toml"
    fi
    if [ "$CURRENT_VERSION" = "$VERSION" ]; then
        err "Cargo.toml is already at version ${VERSION}"
    fi

    say "Preparing release ${TAG} for ${PACKAGE_NAME} (${CURRENT_VERSION} -> ${VERSION})..."

    MODIFIED_MANIFESTS=1
    set_package_version "$VERSION"

    say "Refreshing Cargo.lock..."
    cargo update --workspace --quiet

    say "Running clippy..."
    cargo clippy --locked --all-targets -- -D warnings

    say "Running test suite..."
    cargo test --locked

    if command -v ast-grep >/dev/null 2>&1; then
        say "Running ast-grep YAML snapshot tests..."
        ast-grep test
    else
        say "warning: ast-grep CLI not found; skipping tests/rule_tests/__snapshots__ verification" >&2
    fi

    say "Running ${PACKAGE_NAME} self-check..."
    cargo run --locked -- check .

    say "Verifying crates.io package (dry run)..."
    cargo publish --dry-run --allow-dirty

    if [ "$DRY_RUN" -eq 1 ]; then
        say "Dry run succeeded for ${TAG}. Reverting Cargo.toml and Cargo.lock."
        exit 0
    fi

    say "Committing version bump..."
    git add Cargo.toml Cargo.lock
    git commit -m "chore: release ${TAG}"
    MODIFIED_MANIFESTS=0
    UNPUSHED_RELEASE=1

    say "Verifying clean-tree crates.io package..."
    cargo publish --dry-run

    say "Creating annotated tag ${TAG}..."
    git tag -a "${TAG}" -m "Release ${TAG}"

    say "Atomically pushing ${BRANCH} and ${TAG} to ${REMOTE}..."
    git push --atomic "$REMOTE" "$BRANCH" "refs/tags/${TAG}"
    UNPUSHED_RELEASE=0

    if [ "$SKIP_CRATES" -eq 0 ]; then
        say "Publishing ${PACKAGE_NAME} ${VERSION} to crates.io..."
        if ! cargo publish; then
            err "tag ${TAG} is already pushed to ${REMOTE}, but 'cargo publish' failed. Fix the issue and run: cargo publish"
        fi
    else
        say "Skipping crates.io publish (--skip-crates)."
    fi

    say ""
    say "Released ${PACKAGE_NAME} ${TAG}!"
    if [ -n "$REPO_URL" ]; then
        say "  GitHub Release workflow: ${REPO_URL}/actions/workflows/release.yml"
    fi
    if [ "$SKIP_CRATES" -eq 0 ]; then
        say "  crates.io: https://crates.io/crates/${PACKAGE_NAME}/${VERSION}"
    fi
}

main "$@"
