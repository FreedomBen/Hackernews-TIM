PREFIX      ?= /usr/local
DESTDIR     ?=
BINDIR       = ${DESTDIR}${PREFIX}/bin
DOCDIR       = ${DESTDIR}${PREFIX}/share/doc/hackernews_tim
EXAMPLEDIR   = ${DESTDIR}${PREFIX}/share/hackernews_tim/examples
MANDIR       = ${DESTDIR}${PREFIX}/share/man/man1
MANPAGE      = hackernews_tim.1

CARGO       ?= cargo
BIN_NAME     = hackernews_tim
RELEASE_BIN  = target/release/${BIN_NAME}
DEBUG_BIN    = target/debug/${BIN_NAME}

DOCKER_IMAGE ?= aome510/hackernews_tim
DOCKER_TAG   ?= latest
CROSS_TARGET ?= x86_64-unknown-linux-gnu

.DEFAULT_GOAL := help

.PHONY: help all build debug release run test e2e check fmt fmt-check clippy lint \
        clean install uninstall docker-build docker-run cross-build doc \
        insta-review insta-accept

help: ## Show this help
	@echo "hackernews_tim — Makefile targets"
	@echo ""
	@echo "Usage: make [target] [PREFIX=/usr/local] [DESTDIR=]"
	@echo ""
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z0-9_-]+:.*##/ \
		{ printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2 }' ${MAKEFILE_LIST}
	@echo ""
	@echo "Variables:"
	@echo "  PREFIX        Install prefix (default: /usr/local)"
	@echo "  DESTDIR       Staging dir for packaging (default: empty)"
	@echo "  CARGO         Cargo binary (default: cargo)"
	@echo "  DOCKER_IMAGE  Docker image name (default: aome510/hackernews_tim)"
	@echo "  DOCKER_TAG    Docker tag (default: latest)"
	@echo "  CROSS_TARGET  Cross target triple (default: x86_64-unknown-linux-gnu)"

all: release ## Alias for release

build: release ## Alias for release

debug: ## Build debug binary
	${CARGO} build --workspace

release: ${RELEASE_BIN} ## Build optimized release binary

${RELEASE_BIN}:
	${CARGO} build --workspace --release

run: ## Run the app (debug build)
	${CARGO} run -p hackernews_tim --

test: ## Run workspace tests (enables test-support for integration tests)
	${CARGO} test --workspace --all-features

e2e: ## Run end-to-end PTY tests (Linux-only); each tests/e2e_*.rs binary serially
	@for f in hackernews_tim/tests/e2e_*.rs; do \
		name=$$(basename $$f .rs); \
		echo "==> ${CARGO} test -p hackernews_tim --test $$name -- --test-threads=1"; \
		${CARGO} test -p hackernews_tim --test $$name -- --test-threads=1 || exit 1; \
	done

check: ## Type-check without producing binaries
	${CARGO} check --workspace --all-targets --all-features

fmt: ## Format sources with rustfmt
	${CARGO} fmt --all

fmt-check: ## Verify formatting (CI-friendly)
	${CARGO} fmt --all -- --check

clippy: ## Run clippy lints (warnings as errors)
	${CARGO} clippy --workspace --all-targets --all-features -- -D warnings

lint: fmt-check clippy ## Run fmt-check and clippy

doc: ## Build rustdoc for the workspace
	${CARGO} doc --workspace --no-deps

clean: ## Remove build artifacts
	${CARGO} clean

install: release ## Install binary, man page, docs, and example configs under PREFIX
	install -d "${BINDIR}" "${DOCDIR}" "${EXAMPLEDIR}" "${MANDIR}"
	install -m 0755 "${RELEASE_BIN}" "${BINDIR}/${BIN_NAME}"
	install -m 0644 README.md LICENSE "${DOCDIR}/"
	install -m 0644 docs/config.md "${DOCDIR}/"
	install -m 0644 examples/config.toml examples/config-dark.toml "${EXAMPLEDIR}/"
	install -m 0644 "docs/${MANPAGE}" "${MANDIR}/${MANPAGE}"

uninstall: ## Remove files installed by 'install'
	rm -f "${BINDIR}/${BIN_NAME}" "${MANDIR}/${MANPAGE}"
	rm -rf "${DOCDIR}" "${EXAMPLEDIR}"

docker-build: ## Build Docker image (DOCKER_IMAGE:DOCKER_TAG)
	docker build -t "${DOCKER_IMAGE}:${DOCKER_TAG}" .

docker-run: ## Run Docker image interactively
	docker run --rm -it "${DOCKER_IMAGE}:${DOCKER_TAG}"

cross-build: ## Cross-compile release binary for CROSS_TARGET (uses Cross.toml)
	cross build --release --target ${CROSS_TARGET}

insta-review: ## Review pending insta snapshot diffs (requires cargo-insta)
	${CARGO} insta review

insta-accept: ## Accept all pending insta snapshots without review
	${CARGO} insta accept
