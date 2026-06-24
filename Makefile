.PHONY: check
check: # Run code quality tools
	@echo "Run formatter"
	@cargo fmt
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@checkrs run src/
	@echo "Run flowmark"
	@uvx --from flowmark==0.7.2 flowmark -w 88 --list-spacing tight --nobackup -c --inplace .

.PHONY: bin
bin: # Install binary
	@echo "Install hawk binary"
	@cargo install --path . --locked

.PHONY: test
test: # Run tests
	@echo "Run tests"
	@cargo test --quiet

FIXTURE_DIRS := $(wildcard fixtures/*)

.PHONY: build-fixtures
build-fixtures: # Force-rebuild all test fixtures
	@echo "Building fixtures"
	@for d in $(FIXTURE_DIRS); do \
		echo "  $$d"; \
		forge build --root "$$d" --force --quiet || true; \
	done
