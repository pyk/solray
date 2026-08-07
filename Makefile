.PHONY: lint
lint: # Run linter
	@echo "Run formatter check"
	@cargo fmt --check
	@uvx --from panache-cli==2.61.0 panache format --check .
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@uvx --from git+https://github.com/pyk/checkrs checkrs run src/

.PHONY: fmt
fmt: # Run formatter
	@echo "Run rust formatter"
	@cargo fmt
	@echo "Run markdown formatter"
	@uvx --from panache-cli==2.61.0 panache format .

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
		if [ "$$(basename $$d)" = "function-source" ]; then \
			echo "  $$d (incremental)"; \
			( cd "$$d" && forge clean > /dev/null 2>&1; \
			forge build --quiet 2>/dev/null; \
			echo "// incremental marker" >> src/Incremental.sol; \
			echo "// incremental marker" >> src/CrossFileModifierUser.sol; \
			echo "// incremental marker" >> src/CrossFileModifierBase.sol; \
			forge build --quiet 2>/dev/null; \
			head -n -1 src/Incremental.sol > src/Incremental.sol.tmp \
				&& mv src/Incremental.sol.tmp src/Incremental.sol; \
			head -n -1 src/CrossFileModifierUser.sol > src/CrossFileModifierUser.sol.tmp \
				&& mv src/CrossFileModifierUser.sol.tmp src/CrossFileModifierUser.sol; \
			head -n -1 src/CrossFileModifierBase.sol > src/CrossFileModifierBase.sol.tmp \
				&& mv src/CrossFileModifierBase.sol.tmp src/CrossFileModifierBase.sol; ) \
		else \
			echo "  $$d"; \
			forge build --root "$$d" --force --quiet || true; \
		fi; \
	done
