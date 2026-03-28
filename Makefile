SETTINGS := $(HOME)/.claude/settings.json

.PHONY: install uninstall

install:
	cargo install --path .
	cmd-guard --setup

uninstall:
	@rm -f $(HOME)/.claude/hooks/cmd-guard
	@cargo uninstall cmd-guard 2>/dev/null || true
	@echo "Uninstalled cmd-guard"
	@echo "Note: hook entry in $(SETTINGS) was not removed — edit manually if desired."
