#!/usr/bin/env bash
set -euo pipefail

mode="${1:-all}"
repo_root="$(git rev-parse --show-toplevel)"
debug_bin="$repo_root/target/debug/agent-manager"
verify_temp_root=""
verify_sandbox_dir=""

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "verify-closure: missing required command: $1" >&2
    exit 2
  fi
}

binary_version() {
  "$1" --version | awk '{print $2}'
}

source_checks() {
  echo "verify-closure[source]: workspace tests"
  (
    cd "$repo_root"
    cargo test --workspace
    cargo build -p agent-manager-cli
  )

  echo "verify-closure[source]: GUI tests and build"
  (
    cd "$repo_root/apps/agent-manager-gui"
    npm run test
    npm run build
  )
}

sandbox_checks() {
  require_command jq

  if [[ ! -x "$debug_bin" ]]; then
    (
      cd "$repo_root"
      cargo build -p agent-manager-cli
    )
  fi

  local sandbox_home asset_root
  verify_temp_root="${TMPDIR:-/tmp}"
  verify_sandbox_dir="$(mktemp -d "${verify_temp_root%/}/agent-manager-verify.XXXXXX")"
  sandbox_home="$verify_sandbox_dir/home"
  asset_root="$sandbox_home/.agent-manager"

  cleanup_sandbox() {
    case "$verify_sandbox_dir" in
      "${verify_temp_root%/}"/agent-manager-verify.*)
        rm -rf -- "$verify_sandbox_dir"
        ;;
      *)
        echo "verify-closure: refusing to clean unexpected path: $verify_sandbox_dir" >&2
        ;;
    esac
  }
  trap cleanup_sandbox EXIT INT TERM

  mkdir -p "$asset_root/skills/closure-probe"
  printf '%s\n' \
    '---' \
    'name: closure-probe' \
    'description: isolated agent-manager lifecycle probe' \
    '---' \
    '# Closure probe' \
    >"$asset_root/skills/closure-probe/SKILL.md"

  local -a isolated_env
  isolated_env=(
    "HOME=$sandbox_home"
    "USERPROFILE=$sandbox_home"
    "AGENT_MANAGER_ROOT=$asset_root"
    "AGENT_MANAGER_SECRETS_DIR=$sandbox_home/.config/agent-manager"
    "HERMES_SKILLS_DIR=$sandbox_home/.hermes/skills"
  )

  echo "verify-closure[sandbox]: plan is read-only"
  local plan
  plan="$(env "${isolated_env[@]}" "$debug_bin" --json sync)"
  jq -e '.plan.schema_version >= 1 and (.plan.actions | length) > 0' <<<"$plan" >/dev/null
  jq -e '.blocking_reason == null' <<<"$plan" >/dev/null
  test ! -e "$sandbox_home/.agents/skills/closure-probe"
  test ! -e "$sandbox_home/.claude/skills/closure-probe"
  test ! -e "$sandbox_home/.hermes/config.yaml"

  echo "verify-closure[sandbox]: explicit apply creates supported targets"
  env "${isolated_env[@]}" "$debug_bin" --json sync --apply >/dev/null
  test -e "$sandbox_home/.agents/skills/closure-probe/SKILL.md"
  test -e "$sandbox_home/.claude/skills/closure-probe/SKILL.md"
  grep -Fq "$asset_root/skills/closure-probe" "$sandbox_home/.hermes/config.yaml"

  local status
  status="$(env "${isolated_env[@]}" "$debug_bin" --json status)"
  jq -e '
    .summary.broken == 0
    and .summary.wrong_source == 0
    and .summary.wrong_type == 0
  ' <<<"$status" >/dev/null

  echo "verify-closure[sandbox]: uninstall is reviewed and reversible"
  env "${isolated_env[@]}" "$debug_bin" --json uninstall >/dev/null
  test -e "$sandbox_home/.agents/skills/closure-probe/SKILL.md"
  env "${isolated_env[@]}" "$debug_bin" --json uninstall --apply >/dev/null
  test ! -e "$sandbox_home/.agents/skills/closure-probe"
  test ! -e "$sandbox_home/.claude/skills/closure-probe"
  test -e "$sandbox_home/.hermes/config.yaml"
  if grep -Fq "$asset_root/skills/closure-probe" "$sandbox_home/.hermes/config.yaml"; then
    echo "verify-closure: Hermes external skill remained after uninstall" >&2
    exit 1
  fi
  test -e "$asset_root/skills/closure-probe/SKILL.md"

  echo "verify-closure[sandbox]: diagnostics and completion"
  env "${isolated_env[@]}" "$debug_bin" --json doctor |
    jq -e '.broken == 0 and (.missing_secrets | length) == 0' >/dev/null
  env "${isolated_env[@]}" "$debug_bin" completion zsh >/dev/null
  set -o pipefail
  env "${isolated_env[@]}" "$debug_bin" completion zsh \
    2>"$verify_sandbox_dir/completion.stderr" |
    head -n 2 >/dev/null
  if grep -q 'panicked\\|Broken pipe' "$verify_sandbox_dir/completion.stderr"; then
    echo "verify-closure: completion emitted a broken-pipe panic" >&2
    exit 1
  fi

  cleanup_sandbox
  trap - EXIT INT TERM
  echo "verify-closure[sandbox]: PASS"
}

runtime_checks() {
  require_command jq

  if [[ ! -x "$debug_bin" ]]; then
    echo "verify-closure[runtime]: build the debug binary first" >&2
    exit 2
  fi

  local installed_bin built_version installed_version
  installed_bin="$(command -v agent-manager || true)"
  if [[ -z "$installed_bin" ]]; then
    echo "verify-closure[runtime]: agent-manager is not installed on PATH" >&2
    exit 3
  fi

  built_version="$(binary_version "$debug_bin")"
  installed_version="$(binary_version "$installed_bin")"
  echo "verify-closure[runtime]: built=$debug_bin@$built_version"
  echo "verify-closure[runtime]: installed=$installed_bin@$installed_version"
  if [[ "$built_version" != "$installed_version" ]]; then
    echo "verify-closure[runtime]: version drift blocks runtime acceptance" >&2
    exit 4
  fi

  local doctor status secrets runtime_failed
  runtime_failed=0
  doctor="$("$installed_bin" --json doctor)"
  echo "verify-closure[runtime]: doctor summary"
  jq '{
    broken,
    wrong_source,
    wrong_type,
    missing_secret_count:(.missing_secrets | length),
    literal_mcp_secret_values:(.literal_mcp_secret_values // 0),
    legacy_mcp_secret_file_count:((.legacy_mcp_secret_files // []) | length)
  }' <<<"$doctor"
  if ! jq -e '
    .broken == 0
    and .wrong_source == 0
    and .wrong_type == 0
    and (.missing_secrets | length) == 0
    and (.literal_mcp_secret_values // 0) == 0
  ' <<<"$doctor" >/dev/null; then
    echo "verify-closure[runtime]: doctor blockers remain" >&2
    runtime_failed=1
  fi

  status="$("$installed_bin" --json status)"
  echo "verify-closure[runtime]: status summary"
  jq '.summary' <<<"$status"
  if ! jq -e '
    .summary.broken == 0
    and .summary.wrong_source == 0
    and .summary.wrong_type == 0
  ' <<<"$status" >/dev/null; then
    echo "verify-closure[runtime]: platform drift remains" >&2
    runtime_failed=1
  fi

  secrets="$("$installed_bin" --json secrets validate)"
  if ! jq -e '.ok == true and (.missing | length) == 0' <<<"$secrets" >/dev/null; then
    echo "verify-closure[runtime]: secret references are missing" >&2
    runtime_failed=1
  fi

  if (( runtime_failed != 0 )); then
    exit 1
  fi

  echo "verify-closure[runtime]: PASS (read-only)"
}

case "$mode" in
  source)
    source_checks
    ;;
  sandbox)
    sandbox_checks
    ;;
  runtime)
    runtime_checks
    ;;
  all)
    source_checks
    sandbox_checks
    runtime_checks
    ;;
  *)
    echo "usage: $0 [source|sandbox|runtime|all]" >&2
    exit 2
    ;;
esac
