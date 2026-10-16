#!/usr/bin/env bash
# Decides whether a scheduled (or manually dispatched) test run has any work to
# do (AG-902). It finds the most recent run of the calling workflow on
# GUARD_BRANCH whose suite jobs all concluded success, and says "run" when:
#   - FORCE is true, or
#   - no such run exists, or
#   - that run tested a different commit than HEAD_SHA, or
#   - that run started more than FLOOR_DAYS days ago.
# Otherwise it says "skip" and writes one line to the job summary naming the
# commit it last proved, when, and when the next floor run is due.
# If the newest run that executed the suite failed, it also says "run", so a
# red night retries the next one even when an older pass covers the commit.
#
# Proof is the suite jobs, not the run's conclusion: a run whose suite was
# skipped by this guard still concludes success, and must not count.
# pull_request runs never count either: they test GitHub's merge commit, not
# the branch head.
#
# Env:
#   GH_TOKEN            token with actions:read
#   GITHUB_REPOSITORY   owner/repo
#   WORKFLOW_FILE       workflow file name, e.g. e2e-real-tools.yml
#   GUARD_BRANCH        branch whose runs count as proof
#   HEAD_SHA            commit this run would test
#   SUITE_JOB_PREFIX    name prefix of the suite jobs, e.g. "real-tools ("
#   FLOOR_DAYS          re-run at least this often (default 7)
#   FORCE               "true" runs regardless
#   NOW                 epoch seconds; defaults to now (for tests)
# Writes run=true|false to $GITHUB_OUTPUT when set, else prints it.
set -euo pipefail

: "${GITHUB_REPOSITORY:?}" "${WORKFLOW_FILE:?}" "${GUARD_BRANCH:?}" "${HEAD_SHA:?}" "${SUITE_JOB_PREFIX:?}"
FLOOR_DAYS="${FLOOR_DAYS:-7}"
NOW="${NOW:-$(date -u +%s)}"
OUT="${GITHUB_OUTPUT:-/dev/stdout}"
SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/stdout}"
SERVER="${GITHUB_SERVER_URL:-https://github.com}"

decide() { # run|skip, reason
  echo "run-guard: $2"
  if [ "$1" = run ]; then echo "run=true" >>"$OUT"; else echo "run=false" >>"$OUT"; fi
}

if [ "${FORCE:-false}" = true ]; then
  decide run "forced; running the suite"
  exit 0
fi

# Walk completed runs newest first and stop at the first one whose suite jobs
# actually ran: all success is the proof, anything else means the last attempt
# failed (or was cancelled) and this run retries. Runs where the guard skipped
# the suite have no executed suite job and are passed over.
proof=""
while read -r id sha started; do
  [ -n "$id" ] || continue
  verdict="$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$id/jobs?per_page=100" \
    | jq -r --arg p "$SUITE_JOB_PREFIX" \
      '[.jobs[] | select(.name | startswith($p)) | .conclusion | select(. != "skipped")]
       | if length == 0 then "none" elif all(. == "success") then "pass" else "fail" end')"
  case "$verdict" in
    pass) proof="$id $sha $started"; break ;;
    fail)
      decide run "the last suite on $GUARD_BRANCH (run $id, ${sha:0:7}) did not pass; retrying"
      exit 0 ;;
  esac
done < <(gh api "repos/$GITHUB_REPOSITORY/actions/workflows/$WORKFLOW_FILE/runs?branch=$GUARD_BRANCH&status=completed&per_page=30" \
  -q '.workflow_runs[] | select(.event != "pull_request") | "\(.id) \(.head_sha) \(.run_started_at)"')

if [ -z "$proof" ]; then
  decide run "no passing suite on $GUARD_BRANCH yet; running the suite"
  exit 0
fi

read -r p_id p_sha p_started <<<"$proof"
started_s="$(jq -nr --arg t "$p_started" '$t | fromdateiso8601')"
next_s=$((started_s + FLOOR_DAYS * 86400))
p_date="$(jq -nr --argjson s "$started_s" '$s | strftime("%Y-%m-%d %H:%M UTC")')"
next_date="$(jq -nr --argjson s "$next_s" '$s | strftime("%Y-%m-%d")')"

if [ "$p_sha" != "$HEAD_SHA" ]; then
  decide run "$GUARD_BRANCH moved: last proved ${p_sha:0:7} (run $p_id), head is ${HEAD_SHA:0:7}; running the suite"
elif [ "$NOW" -ge "$next_s" ]; then
  decide run "${p_sha:0:7} last proved $p_date (run $p_id), over $FLOOR_DAYS days ago; running the floor suite"
else
  decide skip "${p_sha:0:7} already passed in run $p_id"
  echo "Skipped: \`${p_sha:0:7}\` already passed on $p_date ([run $p_id]($SERVER/$GITHUB_REPOSITORY/actions/runs/$p_id)); the next $FLOOR_DAYS-day floor run is due $next_date." >>"$SUMMARY"
fi
