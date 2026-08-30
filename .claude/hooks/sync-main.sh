#!/bin/sh
# PostToolUse hook on Bash: after a `gh pr merge`, keep the user's
# main checkout current — METHODOLOGIES.md "After a PR merges",
# step 1, done by the harness instead of by hand. Hooks run as the
# user outside the worktree sandbox, so an isolated session's merge
# still lands in the shared checkout. Never blocks: every path exits 0
# and reports through systemMessage.
set -u

command=$(jq -r '.tool_input.command // ""' 2>/dev/null)
case "$command" in
  *"gh pr merge"*) ;;
  *) exit 0 ;;
esac

say() { jq -n --arg m "$1" '{systemMessage: $m}'; }

root=$(git -C "${CLAUDE_PROJECT_DIR:-.}" worktree list --porcelain 2>/dev/null \
  | sed -n '1s/^worktree //p')
[ -n "$root" ] || exit 0

if ! out=$(git -C "$root" fetch --quiet origin 2>&1); then
  say "main checkout ($root): fetch from origin failed — not updated: $out"
  exit 0
fi

branch=$(git -C "$root" symbolic-ref --short -q HEAD || echo "(detached)")
before=$(git -C "$root" rev-parse --short main 2>/dev/null || echo "?")
if [ "$branch" = "main" ]; then
  if ! out=$(git -C "$root" merge --ff-only --quiet origin/main 2>&1); then
    say "main checkout ($root) not fast-forwarded: $out"
    exit 0
  fi
else
  # The checkout is on another branch: move only the local `main`
  # ref (fast-forward only) so it is current when they switch back.
  if ! out=$(git -C "$root" fetch --quiet origin main:main 2>&1); then
    say "local main not fast-forwarded (checkout is on $branch): $out"
    exit 0
  fi
fi
after=$(git -C "$root" rev-parse --short main)
if [ "$before" != "$after" ]; then
  say "main checkout fast-forwarded $before → $after ($root, on $branch)"
fi
exit 0
