# Gate Connect: Cognito OAuth client setup (runbook)

Gate Connect authenticates to the gateway with a Cognito **access token** on
the `x-gate-authorization` header (the pasted API key on `x-gate-api-key` is the
legacy fallback). To do that it needs a dedicated Cognito **app client** in the
same user pool Gate uses.

That client is created **out-of-band** (not managed by Gate's Terraform). Gate's
Terraform consumes its id through the `cognito_desktop_client_id` variable:

- `variables.tf` declares `cognito_desktop_client_id`
- `security.tf` sets `local.cognito.connect_client_id = var.cognito_desktop_client_id`
- `compute.tf` puts it in the gateway env:
  `GATEWAY_COGNITO_CLIENT_IDS = join(",", compact([client_id, connect_client_id]))`
- each env's tfvars supplies the value (see `staging.tfvars.example`)

This runbook is the source of truth for creating and re-creating that client per
environment. Run it once per environment (staging, production), and again if the
client is ever rebuilt.

## Why out-of-band and not a Terraform resource

The desktop client differs from the browser SPA clients in the shared `cognito`
module: it is a public client (no secret), uses loopback callbacks, and carries
its own managed-login branding and logo asset. Keeping it out-of-band avoids
vendoring branding assets into the module and keeps the loopback specifics out of
the SPA client config. Terraform still owns the gateway trust wiring; it just
takes the client id as an input.

## Prerequisites

1. **Correct AWS account and region.** This is the number one failure. Wrong
   creds show up as `describe-user-pool-client` returning "does not exist" and
   the Hosted UI showing "Login pages unavailable. Please contact an
   administrator." Verify first:
   ```bash
   aws sts get-caller-identity
   ```
2. **Disable the AWS CLI v2 pager** for this session, or output gets swallowed in
   a non-interactive shell:
   ```bash
   export AWS_PAGER=""
   ```
3. The target pool is Terraform-managed (`cognito_manage_pool = true`), so the
   pool, domain, and SPA clients already exist. This runbook only adds the
   Connect client and its branding.

## Reference values

| Item | Staging value |
|---|---|
| Region | `us-east-1` |
| User pool id | `us-east-1_GPcJkAGzM` |
| Hosted domain | `swarm-deck-staging-ue1.auth.us-east-1.amazoncognito.com` |
| Domain prefix | `swarm-deck-staging-ue1` |

App-side constants (from `crates/core/src/oauth.rs`):

| Item | Value |
|---|---|
| Loopback callback ports (`REDIRECT_PORTS`) | `8977`, `8978`, `8979` |
| Callback path | `/callback` (host `localhost`, scheme `http`) |
| Scopes (default) | `openid email profile aws.cognito.signin.user.admin` |

For production, resolve the equivalent pool id and domain with step 1 against the
production account and region.

## Step 1: Confirm the pool behind the hosted domain

Always work back from the domain so the client lands in the pool the domain
fronts (a client in a different pool causes "Login pages unavailable"):

```bash
aws cognito-idp describe-user-pool-domain --domain swarm-deck-staging-ue1 \
  --region us-east-1 --no-cli-pager \
  --query 'DomainDescription.{Pool:UserPoolId,Status:Status,Version:ManagedLoginVersion}'
```

Confirm `Pool` matches the user pool id above. `Version: 2` means the domain uses
**Managed Login**, which makes step 3 mandatory.

## Step 2: Create the dedicated public client

```bash
aws cognito-idp create-user-pool-client \
  --user-pool-id us-east-1_GPcJkAGzM \
  --client-name gate-connect-desktop \
  --no-generate-secret \
  --allowed-o-auth-flows code \
  --allowed-o-auth-flows-user-pool-client \
  --allowed-o-auth-scopes openid email profile aws.cognito.signin.user.admin \
  --callback-urls \
    http://localhost:8977/callback \
    http://localhost:8978/callback \
    http://localhost:8979/callback \
  --supported-identity-providers COGNITO Google \
  --explicit-auth-flows ALLOW_REFRESH_TOKEN_AUTH ALLOW_USER_AUTH \
  --prevent-user-existence-errors ENABLED \
  --region us-east-1 --no-cli-pager --query 'UserPoolClient.ClientId'
```

Record the returned **ClientId**.

Flag notes:
- `--no-generate-secret`: public client (PKCE, no secret), which is what a native
  desktop app is.
- All three loopback callbacks: the app tries `8977`, `8978`, `8979` in order and
  binds the first free one. Cognito matches the callback exactly (scheme, host,
  port, path), so all three must be registered or a port fallback fails.
- Host is `localhost`, not `127.0.0.1`: Cognito only accepts `http` callbacks on
  `localhost`. The app advertises `http://localhost:<port>/callback` accordingly.
- `ALLOW_REFRESH_TOKEN_AUTH`: needed for the app's silent token refresh.
- `ALLOW_USER_AUTH`: enables choice-based sign-in, which is what surfaces the
  passkey option in Managed Login (the shared pool's WebAuthn config does the
  rest). This necessarily switches the login screen to the identifier-first
  design (email first, then password/passkey on the next screen); the classic
  combined username+password screen and passkey are mutually exclusive. Omit this
  flag to keep the classic screen at the cost of no passkey.
- `COGNITO Google`: lets users who signed up via Google sign in too.
- Scopes match the app's compiled default, so no build-time scope override is
  needed. Optional hardening below.

## Step 3: Create managed-login branding (required when domain is v2)

A Managed Login (v2) domain serves "Login pages unavailable" for any client that
has no published branding style. Create a baseline first so login renders:

```bash
aws cognito-idp create-managed-login-branding \
  --user-pool-id us-east-1_GPcJkAGzM --client-id <CONNECT_CLIENT_ID> \
  --use-cognito-provided-values --region us-east-1 --no-cli-pager \
  --query 'ManagedLoginBranding.ManagedLoginBrandingId'
```

Record the **ManagedLoginBrandingId**. At this point the Hosted UI renders with
Cognito's stock look. To make it read as Gate, apply the `cg` brand palette and
logo below.

> **Trap: do not export the settings from a `--use-cognito-provided-values`
> branding and edit them.** That baseline stores **no** settings document, so
> `describe-managed-login-branding ... --query 'ManagedLoginBranding.Settings'`
> returns `null`. Feeding `null` back with `--no-use-cognito-provided-values`
> makes `update-managed-login-branding` fail with a 500
> (`InternalErrorException`), because Cognito has an empty branding to render.
> You need a complete, valid settings document first (next paragraph).

Brand values (resolved from `gate/packages/frontend-ui/src/cg/tokens.css`):

| Role | cg token | Value |
|---|---|---|
| Primary (button bg, text) | `ink-900` / white | `#020202` on `#ffffff` |
| Primary hover | `ink-800` | `#0e0e0e` |
| Muted text | `ink-500` | `#6c6c6c` |
| Borders / link underline | `ink-200` | `#e1e1e1` |
| Page background | `canvas` | `#ecece7` |
| Card / form surface | `white` | `#ffffff` |
| Control radius / card radius | `radius-md` / modal | 8px / 12px |
| Color scheme mode | light only | `LIGHT` |

Note: Managed Login offers only a curated font list, so the type is a near-match
to Geist, not literal Geist.

### Applying the brand (Console designer, recommended)

Author the style in the **Console Managed Login designer**, which generates a
valid settings document, handles the logo upload, and previews live. Do not
hand-author the settings JSON; that is what triggers the 500 above.

1. Cognito, your user pool, **Managed login**, edit the style for the
   `gate-connect-desktop` client.
2. Apply the palette in the table, set **LIGHT** mode, and upload
   `docs/cognito/logo-light.png` as the form logo (enable the logo toggle).
3. Save and publish.

### Making it reproducible (optional, CLI)

Once the designer style exists, its settings are real (not `null`), so you can
export them and vendor them for repeatable applies in other environments:

```bash
aws cognito-idp describe-managed-login-branding \
  --user-pool-id us-east-1_GPcJkAGzM --managed-login-branding-id <ID> \
  --region us-east-1 --no-cli-pager --query 'ManagedLoginBranding.Settings' \
  > docs/cognito/connect-branding-settings.json

# re-apply (e.g. in another env), run from the repo root:
aws cognito-idp update-managed-login-branding \
  --user-pool-id <POOL_ID> --managed-login-branding-id <ID> \
  --settings file://docs/cognito/connect-branding-settings.json \
  --assets file://docs/cognito/connect-branding-assets.json \
  --no-use-cognito-provided-values \
  --region <REGION> --no-cli-pager
```

> **Always send `--assets` together with `--settings`.** `update-managed-login-branding`
> replaces the branding with exactly what you pass, so a settings-only update
> (a quick color tweak) silently drops the logo. Keep both files on every apply,
> and validate the settings JSON first
> (`python3 -m json.tool docs/cognito/connect-branding-settings.json > /dev/null`)
> so a broken edit fails locally instead of as an opaque 500.

### Logo asset

The logo is vendored in this repo so the branding is reproducible without
reaching into the Gate repo:

- `docs/cognito/logo-light.png` - the `cg` brand lockup (dark, for light
  surfaces), copied from `gate/packages/frontend-ui/src/cg/layout/brand/logo-light.png`
- `docs/cognito/connect-branding-assets.json` - that PNG encoded as a Cognito
  `FORM_LOGO` / `LIGHT` / `PNG` asset, which the `update-managed-login-branding`
  command above consumes directly

Regenerate the assets file if the logo changes:

```bash
python3 - <<'PY'
import base64, json
b64 = base64.b64encode(open("docs/cognito/logo-light.png", "rb").read()).decode()
assets = [{"Category": "FORM_LOGO", "ColorMode": "LIGHT", "Extension": "PNG", "Bytes": b64}]
open("docs/cognito/connect-branding-assets.json", "w").write(json.dumps(assets))
PY
```

Enable the form logo in `branding.json` (the "Logo" toggle in the designer), or
the asset will not show.

## Step 4: Wire the client id into Gate's Terraform

In the Gate repo, set the client id in the environment's real tfvars (not the
`.example`):

```hcl
# terraform/aws/environments/staging.tfvars
cognito_desktop_client_id = "<CONNECT_CLIENT_ID>"
```

Then apply. The gateway env `GATEWAY_COGNITO_CLIENT_IDS` already references
`local.cognito.connect_client_id`, so applying redeploys the gateway trusting the
new client. Until this is set, sign-in captures a token but `GET /v1/me/orgs`
(the org picker) and all proxied calls return 401.

## Step 5: Wire the client id into the Connect build

The app reads its OAuth client config, baked at compile time via `option_env!`
in `crates/core/src/oauth.rs`, with process env overriding at runtime. They are
public client config, not secrets.

**Two pools, picked at runtime.** Both the production and staging Cognito pools
are baked into a single binary. `OAuthConfig::from_build_env()` selects the pair
matching the active gateway: if `account.gateway_base_url`'s host equals
`STAGING_GATEWAY_HOST` (`gateway-staging.constellationgate.ai`, kept in sync with
`GATEWAY_SERVERS` in `src/lib/config.ts`) it uses the `_STAGING` values;
every other host (production, self-hosted, unknown, or no account yet) uses the
prod values.

Release builds: set as repo Variables (Settings, Secrets and variables, Actions,
Variables), consumed by `.github/workflows/release.yml`:

- `GATE_COGNITO_CLIENT_ID` = the prod connect client id
- `GATE_COGNITO_HOSTED_DOMAIN` = the prod hosted domain (no scheme, no trailing slash)
- `GATE_COGNITO_SCOPES` = optional; defaults to
  `openid email profile aws.cognito.signin.user.admin`
- `GATE_COGNITO_CLIENT_ID_STAGING` = the staging connect client id
- `GATE_COGNITO_HOSTED_DOMAIN_STAGING` = the staging hosted domain
- `GATE_COGNITO_SCOPES_STAGING` = optional; same default as prod

`crates/core/build.rs` declares `rerun-if-env-changed` for all six so a cached
`target/` cannot ship a stale value.

Local testing (no rebuild needed, runtime env wins over the baked value). Set
the pair matching the gateway you'll select in Settings → Dev mode:

```bash
# Production gateway (default):
export GATE_COGNITO_HOSTED_DOMAIN=<PROD_HOSTED_DOMAIN>
export GATE_COGNITO_CLIENT_ID=<PROD_CONNECT_CLIENT_ID>

# Staging gateway (select "Staging" in Settings → Dev mode):
export GATE_COGNITO_HOSTED_DOMAIN_STAGING=swarm-deck-staging-ue1.auth.us-east-1.amazoncognito.com
export GATE_COGNITO_CLIENT_ID_STAGING=<STAGING_CONNECT_CLIENT_ID>

pnpm tauri dev
```

## Production setup checklist

The condensed run-through of steps 1-5 against the production account. Staging
is already done; production is not. Each item links back to the step with the
full command and flag rationale.

- [ ] **Prod AWS credentials.** `aws sts get-caller-identity` shows the
  production account; `export AWS_PAGER=""`. (Prerequisites)
- [ ] **Resolve the prod pool.** `describe-user-pool-domain` against the
  production domain prefix; record the pool id, hosted domain, and region, and
  note whether `Version` is `2` (Managed Login). (Step 1)
- [ ] **Create the client.** `create-user-pool-client` in the prod pool:
  `gate-connect-desktop`, no secret, code flow, the three loopback callbacks
  (`http://localhost:8977|8978|8979/callback`), `COGNITO Google`,
  `ALLOW_REFRESH_TOKEN_AUTH ALLOW_USER_AUTH`. Record the **ClientId**. (Step 2)
- [ ] **Branding.** Create baseline branding for the new client
  (`--use-cognito-provided-values`), then apply the Gate style: either
  re-apply the vendored `connect-branding-settings.json` +
  `connect-branding-assets.json` via `update-managed-login-branding` (both
  files, always), or redo the Console designer steps. Verify the Hosted UI
  renders with the logo, not "Login pages unavailable." (Step 3)
- [ ] **Gateway trust.** In the Gate repo, set
  `cognito_desktop_client_id = "<PROD_CLIENT_ID>"` in the production tfvars
  and apply. Until this lands, sign-in succeeds but every gateway call 401s.
  (Step 4)
- [ ] **Release variables.** Set the GitHub Actions repo Variables
  `GATE_COGNITO_CLIENT_ID` (prod client id) and `GATE_COGNITO_HOSTED_DOMAIN`
  (prod hosted domain, no scheme); leave `GATE_COGNITO_SCOPES` unset unless
  hardening. The `_STAGING` variables stay as they are. (Step 5)
- [ ] **Cut a release** so the new values are baked into the binary
  (`build.rs` re-runs on env change, so no stale cache).
- [ ] **Verify end to end.** Fresh install pointing at the production gateway:
  sign in via the branded Hosted UI, org picker loads (`GET /v1/me/orgs`
  returns 200), a proxied tool call succeeds. Then flip Settings → Dev mode to
  Staging and confirm staging login still works (the runtime pool selection
  picks the `_STAGING` pair only for `gateway-staging.constellationgate.ai`).

## Gotchas

- **Wrong account or region** is the most common failure and hides as a missing
  client or "Login pages unavailable." Run `aws sts get-caller-identity` first.
- **Exact callback match**: Cognito matches scheme, host, port, and path. Register
  all three loopback ports; host must be `localhost`, path `/callback`.
- **Managed Login v2** needs a published branding style per client, or the Hosted
  UI shows "Login pages unavailable." This is per client, so a new client needs
  its own style (step 3).
- **Scope hardening (optional)**: the app never calls Cognito user self-service
  APIs and the gateway only reads `sub`, so `aws.cognito.signin.user.admin` is
  unnecessary. To tighten, allow only `openid email` on the client and set
  `GATE_COGNITO_SCOPES="openid email"` in the build (or change the default in
  `oauth.rs`).
- **Do not reuse a shared SPA client** for the desktop app in production: it would
  require adding loopback callbacks to a browser client, coupling token lifetimes,
  and losing per-client revocation and audit. Use the dedicated client.
- **`update-user-pool-client` replaces the whole config.** If you ever edit an
  existing client via the CLI, re-specify every field or use the Console, which
  does a read-modify-write.
