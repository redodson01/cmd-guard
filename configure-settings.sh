#!/usr/bin/env bash
set -euo pipefail

SETTINGS="$HOME/.claude/settings.json"
HOOK_CMD="$HOME/.claude/hooks/cmd-guard"

mkdir -p "$(dirname "$SETTINGS")"

# Start from existing settings or empty object
if [[ -f "$SETTINGS" ]]; then
    current=$(cat "$SETTINGS")
else
    current='{}'
fi

# Check if the hook is already configured
if echo "$current" | grep -q "cmd-guard"; then
    echo "Hook already configured in $SETTINGS"
    exit 0
fi

# The hook entry we want to add
hook_entry=$(cat <<INNER
{
  "matcher": "Bash",
  "hooks": [
    {
      "type": "command",
      "command": "$HOOK_CMD"
    }
  ]
}
INNER
)

# Merge: append our hook to .hooks.PreToolUse (create the array if missing)
updated=$(echo "$current" | HOOK_ENTRY="$hook_entry" python3 -c "
import json, sys, os

settings = json.load(sys.stdin)
hook_entry = json.loads(os.environ['HOOK_ENTRY'])

hooks = settings.setdefault('hooks', {})
pre = hooks.setdefault('PreToolUse', [])

# Only add if not already present
if not any('cmd-guard' in h.get('hooks', [{}])[0].get('command', '') for h in pre):
    pre.append(hook_entry)

json.dump(settings, sys.stdout, indent=2)
print()
")

echo "$updated" > "$SETTINGS"
echo "Updated $SETTINGS with cmd-guard hook"
