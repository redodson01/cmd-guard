HOOKS_DIR := $(HOME)/.claude/hooks
CARGO_BIN := $(HOME)/.cargo/bin/cmd-guard
SYMLINK   := $(HOOKS_DIR)/cmd-guard
SETTINGS  := $(HOME)/.claude/settings.json

.PHONY: install uninstall

install: $(SYMLINK) settings

$(CARGO_BIN): Cargo.toml src/main.rs
	cargo install --path .

$(SYMLINK): $(CARGO_BIN)
	@mkdir -p $(HOOKS_DIR)
	@ln -sf $(CARGO_BIN) $(SYMLINK)
	@echo "Symlinked $(SYMLINK) -> $(CARGO_BIN)"

.PHONY: settings
settings:
	@./configure-settings.sh

uninstall:
	@rm -f $(SYMLINK)
	@cargo uninstall cmd-guard 2>/dev/null || true
	@echo "Uninstalled cmd-guard"
	@echo "Note: hook entry in $(SETTINGS) was not removed — edit manually if desired."
