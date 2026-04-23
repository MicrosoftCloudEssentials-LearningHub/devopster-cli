.PHONY: setup local-dev bootstrap build test lint fmt \
        container-build container-test container-run \
        login-github login-azure login-gitlab auth-status \
        help

OS := $(shell uname -s 2>/dev/null || echo Windows)

## setup: Validate Docker, build the image, and open a container shell
setup:
ifeq ($(OS),Darwin)
	@echo "==> Detected macOS"
	@chmod +x ./scripts/setup.sh && ./scripts/setup.sh
else ifeq ($(OS),Linux)
	@echo "==> Detected Linux"
	@chmod +x ./scripts/setup.sh && ./scripts/setup.sh
else
	@echo "==> Detected Windows"
	@powershell -ExecutionPolicy Bypass -File scripts\\setup.ps1
endif

## local-dev: Full local containerized onboarding (bootstrap + devopster setup)
local-dev:
	$(MAKE) container-run ARGS='bash -lc "make bootstrap && devopster setup"'

## bootstrap: In-container first-run bootstrap (install CLI and run tests)
bootstrap:
	cargo fetch
	cargo install --path . --locked --force
	cargo test

## build: Compile the devopster binary
build:
	cargo build

## test: Run all tests
test:
	cargo test

## lint: Run clippy
lint:
	cargo clippy --all-targets --all-features -- -D warnings

## fmt: Format all Rust source files
fmt:
	cargo fmt --all

## login-github: Sign in to GitHub via browser
login-github:
	cargo run -- login github

## login-azure: Sign in to Azure DevOps via browser
login-azure:
	cargo run -- login azure-devops

## login-gitlab: Sign in to GitLab via browser
login-gitlab:
	cargo run -- login gitlab

## auth-status: Show authentication status for all providers
auth-status:
	cargo run -- login status

## container-build: Build the dev container image
container-build:
	docker build --target dev -t devopster-cli-dev .

## container-test: Run tests inside the container
container-test:
	docker build -t devopster-cli-ci .
	docker run --rm devopster-cli-ci cargo test

## container-run: Open a shell inside the container with host credentials mounted
container-run:
	docker build --target dev -t devopster-cli-dev .
	docker run --rm -it \
		-v "$(HOME)/.config/devopster:/root/.config/devopster" \
		-v "$(PWD):/app" \
		-w /app \
		devopster-cli-dev \
		$(if $(ARGS),$(ARGS),bash)

## help: Show available make targets
help:
	@grep -E '^##' Makefile | sed 's/## /  /'
