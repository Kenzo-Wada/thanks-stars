# Commands for development workflows

fmt:
    cargo fmt

fmt-check:
    cargo fmt -- --check

lint:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo pretty-test

check:
    just fmt-check
    just lint
    just test

release-major:
    just _release major

release-minor:
    just _release minor

release-patch:
    just _release patch

_release bump:
    scripts/release.sh {{bump}}
