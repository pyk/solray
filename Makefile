.PHONY: check
check: # Run code quality tools
	@echo "Run rust formatter"
	@cargo fmt
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@checkrs run src/
	@echo "Run markdown formatter"
	@uvx --from panache-cli==2.61.0 panache format --check .

.PHONY: bin
bin: # Install binary
	@echo "Install solray binary"
	@cargo install --path . --locked

.PHONY: test
test: # Run tests
	@echo "Run tests"
	@cargo test --quiet

FIXTURE_DIRS := $(wildcard fixtures/*)

.PHONY: build-fixtures
build-fixtures: # Force-rebuild all test fixtures with incremental sources
	@echo "Building fixtures"
	@for d in $(FIXTURE_DIRS); do \
		if [ "$$(basename $$d)" = "sources" ]; then \
			echo "  $$d (incremental)"; \
			cd "$$d" && forge clean > /dev/null 2>&1; \
			forge build --quiet 2>/dev/null; \
			echo "// incremental marker" >> src/Incremental.sol; \
			forge build --quiet 2>/dev/null; \
			head -n -1 src/Incremental.sol > src/Incremental.sol.tmp \
				&& mv src/Incremental.sol.tmp src/Incremental.sol; \
		else \
			echo "  $$d"; \
			forge build --root "$$d" --force --quiet || true; \
		fi; \
	done
