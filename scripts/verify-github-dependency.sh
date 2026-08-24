#!/bin/sh

set -eu

write_lock=false
case "${1:-}" in
    "") ;;
    --write-lock) write_lock=true ;;
    *)
        echo "usage: $0 [--write-lock]" >&2
        exit 64
        ;;
esac

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/kcm-github-dependency.XXXXXX")
trap 'rm -rf -- "$temporary_root"' EXIT HUP INT TERM

checkout="$temporary_root/checkout"
cargo_home="$temporary_root/cargo-home"
mkdir -p "$checkout" "$cargo_home"

# git archive contains only tracked files, so the ignored local path patch in
# .cargo/config.toml cannot affect this build. Cargo must also be started from
# the archive itself: --manifest-path alone still discovers config from the
# caller's working directory.
git -C "$repository_root" archive HEAD | tar -x -C "$checkout"

(
    cd "$checkout"
    CARGO_HOME="$cargo_home" cargo update -p kako-craft-lib
    CARGO_HOME="$cargo_home" cargo fmt --all -- --check
    CARGO_HOME="$cargo_home" cargo test --locked
    CARGO_HOME="$cargo_home" cargo clippy --locked --all-targets --all-features -- -D warnings
)

if ! cmp -s "$repository_root/Cargo.lock" "$checkout/Cargo.lock"; then
    if [ "$write_lock" = true ]; then
        cp "$checkout/Cargo.lock" "$repository_root/Cargo.lock"
        echo "Git dependency verified; refreshed tracked Cargo.lock."
    else
        echo "Git dependency verification passed, but Cargo.lock needs refreshing." >&2
        echo "Rerun with --write-lock to install the verified lockfile." >&2
        exit 2
    fi
else
    echo "GitHub dependency and tracked Cargo.lock verified successfully."
fi
