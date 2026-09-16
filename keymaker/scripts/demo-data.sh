#!/usr/bin/env bash
# Demo data for looking at the UI with realistic density.
#
# Every value is obviously fake and every reference is real-looking, so the
# grouping, counts and filtering can be judged against something like what a
# person would actually have.
#
#   ./scripts/demo-data.sh seed     add them
#   ./scripts/demo-data.sh clear    take them all away again
#   ./scripts/demo-data.sh list     show what it would touch
set -euo pipefail

KEYMAKER="${KEYMAKER:-keymaker}"

REFS=(
  # A payments provider, where the read/write split matters.
  stripe/sk_live stripe/sk_test stripe/pk_live stripe/pk_test
  stripe/webhook_secret stripe/connect_client_id stripe/restricted_readonly
  stripe/terminal_token stripe/radar_key

  github/token github/app_private_key github/app_id github/webhook_secret
  github/oauth_client_id github/oauth_client_secret github/packages_token
  github/actions_pat github/codespaces_token github/enterprise_token

  openai/api_key openai/api_key_staging openai/org_id openai/project_id
  openai/realtime_key openai/assistants_key openai/fine_tune_key
  openai/batch_key

  anthropic/api_key anthropic/api_key_dev anthropic/workspace_id
  anthropic/admin_key anthropic/batch_key anthropic/eval_key
  anthropic/mcp_token anthropic/console_token

  cloudflare/api_token cloudflare/account_id cloudflare/zone_id
  cloudflare/r2_access_key cloudflare/r2_secret cloudflare/images_token
  cloudflare/workers_token cloudflare/tunnel_token cloudflare/turnstile_secret
  cloudflare/d1_token

  aws/access_key_id aws/secret_access_key aws/session_token aws/role_arn
  aws/ses_smtp_user aws/ses_smtp_password aws/s3_bucket_key
  aws/cloudfront_key_id aws/kms_key_arn

  hardroad/db_url hardroad/db_url_replica hardroad/redis_url
  hardroad/jwt_signing_key hardroad/session_secret hardroad/smtp_password
  hardroad/sentry_dsn hardroad/admin_token

  # No issuer: these file under `general`.
  scratch_token laptop_backup_key vpn_shared_secret
)

case "${1:-}" in
  seed)
    n=0
    for ref in "${REFS[@]}"; do
      # Recognisably fake, and long enough that redaction has something to do.
      printf 'demo_%s_0000000000000000' "${ref//\//_}" | "$KEYMAKER" set "$ref" >/dev/null
      n=$((n + 1))
    done
    echo "seeded $n demo references"
    echo "remove them with: $0 clear"
    ;;
  clear)
    n=0
    for ref in "${REFS[@]}"; do
      if "$KEYMAKER" rm "$ref" >/dev/null 2>&1; then n=$((n + 1)); fi
    done
    echo "removed $n demo references"
    ;;
  list)
    printf '%s\n' "${REFS[@]}"
    echo "---"
    # Map each reference to its issuer, defaulting to `general`. Order matters:
    # stripping the slash first would make every line match the "no slash" case.
    groups=$(printf '%s\n' "${REFS[@]}" | sed 's|^[^/]*$|general/&|' | cut -d/ -f1 | sort -u)
    echo "${#REFS[@]} references across $(echo "$groups" | wc -l | tr -d ' ') groups:"
    echo "$groups" | sed 's/^/  /' 
    ;;
  *)
    echo "usage: $0 {seed|clear|list}" >&2
    exit 1
    ;;
esac
