import { useId } from "react";
import type { ReactNode } from "react";
import { DEVICE_NAME_MAX_LENGTH } from "../../lib/api";
import { ConstellationHexMark } from "./ConstellationHexMark";
import { Icon } from "./Icon";
import { ModalOption } from "./Modal";
import { BaseSwitch } from "./base";
import { Topbar } from "./Topbar";
import type { TopnavAction } from "./Topbar";

/**
 * The screens that run before there is anything to navigate: sign-in, choosing
 * an organization, and the connected confirmation.
 *
 * The `Flows / Auth` page landed on 2026-08-19 and these follow it: sign-in,
 * the key route as a destination of its own, choosing an organization, naming
 * the device, and the connected confirmation. They were built provisionally
 * before that page existed; what is here now is the drawn flow, minus the
 * `Auth / Error states` section, which carries no ready mark.
 * See plans/new-app-ui-figma.md.
 *
 * No sidebar: there is nothing to navigate to yet, and showing an empty rail
 * would advertise a list the user cannot populate.
 *
 * Presentational. The OAuth and API-key flows stay in the container; these take
 * values and callbacks.
 */

export function SetupLayout({
  menuOpen,
  onMenuToggle,
  onMenuSelect,
  progress,
  children,
}: {
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: TopnavAction) => void;
  /** How far through setup this pane sits, 0..1. Every drawn Setup frame
   * carries the rail under the topbar (`Flows / Setup`, read 2026-08-21), the
   * same element the onboarding tour has. Omitted draws no rail. */
  progress?: number;
  children: ReactNode;
}) {
  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-base-card">
      <Topbar
        menuOpen={menuOpen}
        onMenuToggle={onMenuToggle}
        onMenuSelect={onMenuSelect}
      />
      {progress !== undefined && (
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(progress * 100)}
          aria-label="Setup progress"
          className="h-2 shrink-0 border-b border-base-border bg-base-background"
        >
          {/* Identical to `screens/Onboarding.tsx`'s rail, and deliberately so:
           * the frames draw one element. `#7195FF` under a left-to-right
           * black-to-white 64% wash, which composites navy to pale blue. */}
          <div
            className="h-full transition-[width] duration-300"
            style={{
              width: `${progress * 100}%`,
              background:
                "linear-gradient(90deg, rgba(0,0,0,0.64) 0%, rgba(255,255,255,0.64) 100%), #7195FF",
            }}
          />
        </div>
      )}
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto bg-base-background p-6">
        {/* 496px, r16, shadow/lg. Unpadded: the panes that carry an action
         * footer need it to span the card and sit on a rule, so padding is the
         * body's job (`SetupBody`), not the shell's. */}
        <div className="w-full max-w-[496px] overflow-hidden rounded-2xl border border-base-border bg-base-card shadow-base-lg">
          {children}
        </div>
      </div>
    </div>
  );
}

/** The padded region of a setup card. Separate from the card so a pane can put
 *  a full-bleed action footer under it. */
function SetupBody({ children, gap = 6 }: { children: ReactNode; gap?: 5 | 6 }) {
  return (
    <div className={`flex flex-col p-6 ${gap === 5 ? "gap-5" : "gap-6"}`}>{children}</div>
  );
}

/** The tile every setup header fronts itself with: white, a hairline, and the
 *  design's inset pair - a glow up from the bottom edge, white down from the
 *  top. `brand` swaps the hairline for `blue-ribbon-300` and tints the glow,
 *  which the frames reserve for the ones holding the product mark. */
function HeaderTile({
  size,
  brand,
  children,
}: {
  size: 32 | 48;
  brand?: boolean;
  children: ReactNode;
}) {
  return (
    <span
      aria-hidden
      className={`flex shrink-0 items-center justify-center border bg-base-card text-base-foreground ${
        size === 48 ? "size-12 rounded-md" : "size-8 rounded-sm"
      } ${brand ? "border-blue-ribbon-300" : "border-base-border"}`}
      style={{
        boxShadow: `0 1px 0 0 rgba(0,0,0,0.05), inset 0 -4px 8px 0 ${
          brand ? "rgba(151,195,255,0.24)" : "rgba(0,0,0,0.08)"
        }, inset 0 4px 8px 0 rgba(255,255,255,0.4)`,
      }}
    >
      {children}
    </span>
  );
}

/**
 * Two archetypes, both drawn in `Flows / Setup`:
 *
 * - **centred** (`row` unset) - a 48px tile over a centred title and subtitle.
 *   The entry and exit panes: sign-in, the key form, the diagnostics step.
 * - **row** - a 32px tile beside the title, subtitle left-aligned beneath.
 *   The stepped panes, which pair it with a `SetupFooter`.
 *
 * Both stack at a 12px rhythm, which is what the frames measure.
 */
function SetupHeader({
  title,
  subtitle,
  mark,
  row,
  brand,
  large,
}: {
  title: ReactNode;
  subtitle: ReactNode;
  mark?: ReactNode;
  /** Left-aligned tile-beside-title, for a pane with a footer. */
  row?: boolean;
  /** The tile holds the product mark rather than a glyph. */
  brand?: boolean;
  /** The sign-in wordmark, which the frame sets at 24px rather than 20. */
  large?: boolean;
}) {
  const heading = (
    <h1
      className={`tracking-heading text-base-foreground ${
        large ? "text-2xl font-medium leading-6" : "text-xl font-medium leading-6"
      }`}
    >
      {title}
    </h1>
  );

  if (row) {
    return (
      <header className="flex flex-col gap-3">
        <div className="flex items-center gap-3">
          <HeaderTile size={32} brand={brand}>
            {mark}
          </HeaderTile>
          {heading}
        </div>
        <p className="text-sm leading-5 text-base-muted-foreground">{subtitle}</p>
      </header>
    );
  }

  return (
    <header className="flex flex-col items-center gap-3 text-center">
      <HeaderTile size={48} brand={brand}>
        {mark ?? <ConstellationHexMark size={24} />}
      </HeaderTile>
      {heading}
      <p className="text-sm leading-5 text-base-muted-foreground">{subtitle}</p>
    </header>
  );
}

function PrimaryButton({
  children,
  onClick,
  busy,
  disabled,
  arrow,
}: {
  children: ReactNode;
  onClick: () => void;
  busy?: boolean;
  /** The Auth panes draw their primary muted until the field below it is filled
   *  and the org list has a selection. Same argument as `ModalButton`. */
  disabled?: boolean;
  /** The drawn primary carries the Button component's right arrow. */
  arrow?: boolean;
}) {
  const inert = busy || disabled;
  return (
    <button
      type="button"
      onClick={inert ? undefined : onClick}
      aria-busy={busy || undefined}
      aria-disabled={inert || undefined}
      className={`flex h-11 w-full items-center justify-center gap-2 rounded-md bg-blue-ribbon-700 px-4 text-sm font-medium text-white transition-colors hover:bg-blue-ribbon-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        inert ? "opacity-70" : ""
      } ${disabled && !busy ? "cursor-not-allowed" : ""}`}
    >
      {children}
      {arrow && <Icon name="arrowRight" size={20} />}
    </button>
  );
}

/** The quiet text link the Auth panes hang under their primary: "Go back",
 *  "Skip naming", "Use a different account". */
function SetupLink({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="mx-auto text-sm font-medium leading-5 text-base-primary underline underline-offset-2 transition-colors hover:text-blue-ribbon-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
    >
      {children}
    </button>
  );
}

/** `or`, between the two sign-in routes. */
function OrDivider() {
  return (
    <div className="flex items-center gap-[19px]">
      <span aria-hidden className="h-px flex-1 bg-base-border" />
      <span className="text-sm leading-5 text-base-muted-foreground">or</span>
      <span aria-hidden className="h-px flex-1 bg-base-border" />
    </div>
  );
}

function SecondaryButton({
  children,
  onClick,
}: {
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-11 w-full items-center justify-center rounded-md border border-base-input bg-base-card px-4 text-sm font-medium tracking-button-sm text-base-primary shadow-base-btn transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
    >
      {children}
    </button>
  );
}

function TextField({
  label,
  value,
  onChange,
  placeholder,
  mono,
  type = "text",
  clearable,
  maxLength,
}: {
  label: string;
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  mono?: boolean;
  type?: "text" | "password";
  /** Draws the frame's trailing clear button once there is something to clear. */
  clearable?: boolean;
  /** Stops a paste the backend would only truncate. */
  maxLength?: number;
}) {
  const id = useId();
  return (
    <div className="flex flex-col gap-2">
      <label htmlFor={id} className="text-base-xs font-medium leading-4 text-base-foreground">
        {label}
      </label>
      <div className="relative">
        <input
          id={id}
          type={type}
          value={value}
          placeholder={placeholder}
          maxLength={maxLength}
          onChange={(e) => onChange(e.target.value)}
          className={`h-11 w-full rounded-md border border-base-input bg-base-background px-3 text-sm text-base-foreground shadow-base-xs placeholder:text-base-muted-foreground focus:border-base-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
            mono ? "font-mono" : ""
          } ${clearable && value ? "pr-10" : ""}`}
        />
        {clearable && value && (
          <button
            type="button"
            onClick={() => onChange("")}
            aria-label={`Clear ${label.toLowerCase()}`}
            className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-base-muted-foreground transition-colors hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
          >
            <Icon name="circleX" size={16} />
          </button>
        )}
      </div>
    </div>
  );
}

/** Shown for a failure the user can act on. Mirrors the popover's `ErrorNote`.
 *
 *  A `div` rather than a `p`: the note carries `ErrorDetails`, and a `<details>`
 *  is not phrasing content, so a paragraph here would be invalid markup. */
function SetupError({ children }: { children: ReactNode }) {
  return (
    <div
      role="alert"
      className="rounded-md border border-red-200 bg-red-50 p-3 text-sm leading-5 text-red-900"
    >
      {children}
    </div>
  );
}

export interface GatewayChoice {
  label: string;
  url: string;
}

/**
 * The environment picker, for people working on Gate itself.
 *
 * Footer-quiet on purpose, the same call `screens/FirstRun.tsx` makes: the
 * sign-in decision stays a two-option screen, and the gateway is a line of mono
 * under it with one small control. Collapsed until asked for, because for
 * everybody else the answer is already right.
 */
export function GatewayPicker({
  value,
  servers,
  open,
  onOpenChange,
  onSelect,
}: {
  value: string;
  servers: GatewayChoice[];
  open: boolean;
  onOpenChange: (next: boolean) => void;
  onSelect: (url: string) => void;
}) {
  if (!open) {
    return (
      <div className="flex items-baseline justify-center gap-2 text-center">
        <span className="font-mono text-base-xs text-base-muted-foreground">{value}</span>
        <button
          type="button"
          onClick={() => onOpenChange(true)}
          className="-mx-1.5 -my-1.5 px-1.5 py-1.5 text-base-xs font-medium text-neutral-600 underline decoration-base-input underline-offset-2 transition-colors hover:text-base-foreground"
        >
          change
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2 border-t border-base-border pt-4">
      <div className="flex items-center justify-between">
        <span className="font-mono text-base-xs font-medium uppercase tracking-[0.08em] text-base-muted-foreground">
          Gateway server
        </span>
        <button
          type="button"
          onClick={() => onOpenChange(false)}
          className="text-base-xs font-medium text-neutral-600 transition-colors hover:text-base-foreground"
        >
          Hide
        </button>
      </div>
      <div role="radiogroup" aria-label="Gateway server" className="flex flex-col gap-2">
        {servers.map((server) => (
          <ModalOption
            key={server.url}
            initials={server.label.slice(0, 2).toUpperCase()}
            name={server.label}
            meta={server.url}
            selected={server.url === value}
            onSelect={() => onSelect(server.url)}
          />
        ))}
      </div>
    </div>
  );
}

/**
 * Sign-in. OAuth is the primary path; the API-key path stays available because
 * it is the only route forward for a user whose account has no OAuth identity.
 * `reauth` swaps the copy for an expired-session prompt.
 */
export function WelcomePane({
  reauth,
  onSignIn,
  onUseApiKey,
  gateway,
  busy,
  error,
}: {
  reauth?: boolean;
  onSignIn: () => void;
  /** Opens the key pane. The design makes the key its own destination rather
   *  than a form that unfolds under the sign-in buttons, so this navigates. */
  onUseApiKey: () => void;
  /** Slot for the gateway selector, which only dev builds render. Sits in the
   * footer rather than inside the key form: the gateway is saved before the
   * browser flow too, so it governs both paths. */
  gateway?: ReactNode;
  busy?: boolean;
  error?: ReactNode;
}) {
  return (
    <SetupBody>
      <SetupHeader
        brand
        large={!reauth}
        title={
          reauth ? (
            "Session expired"
          ) : (
            <>
              <span className="font-semibold text-base-primary">Gate</span>{" "}
              <span className="font-semibold text-base-muted-foreground">Connect</span>
            </>
          )
        }
        subtitle={
          reauth
            ? "Sign in again to keep routing your apps through Gate."
            : "Sign in once, then choose which AI apps route through Gate. Claude, Codex, OpenCode, and supported apps keep working normally while Gate handles protection underneath."
        }
      />

      {error && <SetupError>{error}</SetupError>}

      <div className="flex flex-col gap-2">
        <PrimaryButton onClick={onSignIn} busy={busy} arrow>
          {reauth ? "Sign in again" : "Continue with Gate account"}
        </PrimaryButton>
        <OrDivider />
        <SecondaryButton onClick={onUseApiKey}>Use an API key</SecondaryButton>
      </div>

      {gateway}
    </SetupBody>
  );
}

/**
 * `Auth / Connect with API key`. Its own pane, with its own account of what
 * happens after it connects, and a way back to the sign-in choice.
 */
export function ApiKeyPane({
  apiKey,
  onApiKeyChange,
  onConnect,
  onGoBack,
  gateway,
  busy,
  error,
}: {
  apiKey: string;
  onApiKeyChange: (next: string) => void;
  onConnect: () => void;
  onGoBack: () => void;
  gateway?: ReactNode;
  busy?: boolean;
  error?: ReactNode;
}) {
  return (
    <SetupBody>
      <SetupHeader
        mark={<Icon name="key" size={24} />}
        title="Use an API key"
        subtitle="Paste a Gate API key from your dashboard. After it connects, you will name this device before choosing which apps are protected."
      />

      {error && <SetupError>{error}</SetupError>}

      <div className="flex flex-col gap-3">
        <TextField
          label="API key"
          value={apiKey}
          onChange={onApiKeyChange}
          placeholder="Enter or paste your API key"
          mono
          type="password"
        />
        <PrimaryButton onClick={onConnect} busy={busy} disabled={!apiKey.trim()} arrow>
          Connect and continue
        </PrimaryButton>
        <SetupLink onClick={onGoBack}>Go back</SetupLink>
      </div>

      {gateway}
    </SetupBody>
  );
}

/**
 * Both routes end here before any app is chosen, which is what the key pane's
 * own copy promises. Skipping is drawn as a first-class exit and leaves the
 * name following the hostname, which is what `device_name: null` already means.
 */
export function NameDevicePane({
  value,
  onChange,
  onContinue,
  onSkip,
  busy,
  error,
}: {
  value: string;
  onChange: (next: string) => void;
  onContinue: () => void;
  onSkip: () => void;
  busy?: boolean;
  error?: ReactNode;
}) {
  return (
    <SetupBody>
      <SetupHeader
        mark={<Icon name="monitor" size={24} />}
        title="Name this device"
        subtitle="Naming will help you tell this device apart from the others connected to your Gate account."
      />

      {error && <SetupError>{error}</SetupError>}

      <div className="flex flex-col gap-3">
        <TextField
          label="Device name"
          value={value}
          onChange={onChange}
          placeholder="Enter a device name"
          maxLength={DEVICE_NAME_MAX_LENGTH}
          clearable
        />
        <PrimaryButton onClick={onContinue} busy={busy} disabled={!value.trim()} arrow>
          Continue
        </PrimaryButton>
        <SetupLink onClick={onSkip}>Skip naming</SetupLink>
      </div>
    </SetupBody>
  );
}

export interface SetupOrganization {
  id: string;
  name: string;
  initials: string;
  meta: string;
}

/**
 * Choosing an organization after sign-in. The container auto-advances when the
 * user belongs to exactly one, so this only renders for a real choice.
 */
export function OrgPickerPane({
  organizations,
  selectedId,
  onSelect,
  onContinue,
  onGoBack,
  onUseDifferentAccount,
  busy,
  error,
}: {
  organizations: SetupOrganization[];
  selectedId?: string;
  onSelect: (id: string) => void;
  onContinue: () => void;
  /** The dead end's way out. An account that is authenticated but owns nothing
   *  to route for cannot go forward, and the design sends it back to sign-in
   *  rather than sideways into the key form. */
  onGoBack: () => void;
  onUseDifferentAccount: () => void;
  busy?: boolean;
  error?: ReactNode;
}) {
  return (
    <SetupBody>
      <SetupHeader
        mark={<Icon name="usersRound" size={24} />}
        title="Choose an organization"
        subtitle="Gate Connect will use the selected organization for routing, activity, and PAYG credits on this device."
      />

      {error && <SetupError>{error}</SetupError>}

      {organizations.length === 0 ? (
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1 rounded-md border border-amber-300 bg-amber-50 p-3 text-center">
            <p className="flex items-center justify-center gap-1.5 text-sm font-medium leading-5 text-amber-900">
              <Icon name="triangleAlert" size={16} />
              No organizations found.
            </p>
            <p className="text-sm leading-5 text-amber-900">
              You will need to setup your first organization through Gate AI before
              continuing to setup Gate Connect.
            </p>
          </div>
          <PrimaryButton onClick={onGoBack}>Go back</PrimaryButton>
          <SetupLink onClick={onUseDifferentAccount}>Use a different account</SetupLink>
        </div>
      ) : (
        <>
          <div role="radiogroup" aria-label="Organization" className="flex flex-col gap-3">
            {organizations.map((org) => (
              <ModalOption
                key={org.id}
                initials={org.initials}
                name={org.name}
                meta={org.meta}
                selected={org.id === selectedId}
                onSelect={() => onSelect(org.id)}
              />
            ))}
          </div>
          <div className="flex flex-col gap-3">
            <PrimaryButton onClick={onContinue} busy={busy} disabled={!selectedId} arrow>
              Continue
            </PrimaryButton>
            <SetupLink onClick={onUseDifferentAccount}>Use a different account</SetupLink>
          </div>
        </>
      )}
    </SetupBody>
  );
}

/**
 * Connected confirmation. When routing is off the primary action finishes the
 * job the copy promises rather than dead-ending in a "Done" - the same argument
 * `screens/Success.tsx` makes.
 */
export function ConnectedPane({
  workspace,
  offerRouting,
  busy,
  onTurnOnRouting,
  onDone,
}: {
  workspace: string;
  /** Routing is available but off, so finishing here would leave it unrouted. */
  offerRouting: boolean;
  busy?: boolean;
  onTurnOnRouting: () => void;
  onDone: () => void;
}) {
  return (
    <SetupBody>
      <SetupHeader
        title="You're connected"
        subtitle={`Gate Connect is signed in to ${workspace}.`}
        mark={<Icon name="circleCheck" size={24} className="text-green-600" />}
      />

      <div className="flex flex-col gap-3">
        {offerRouting ? (
          <>
            <PrimaryButton onClick={onTurnOnRouting} busy={busy}>
              Turn on routing
            </PrimaryButton>
            <SecondaryButton onClick={onDone}>Not now</SecondaryButton>
          </>
        ) : (
          <PrimaryButton onClick={onDone}>Done</PrimaryButton>
        )}
      </div>
    </SetupBody>
  );
}

/**
 * The last step before Overview: the diagnostic-data choice.
 *
 * Placed here rather than in Settings-only because consent belongs before
 * collection, not after it. `lib/analytics.ts` starts PostHog at launch, so the
 * first thing this step buys is a person who has actually been asked.
 *
 * Drawn at last (`Flows / Setup`, "Share diagnostic data", read 2026-08-21): a
 * share2 tile, one sentence of copy, the sharing row with its switch, a
 * `Finish setup` primary and a `Skip data sharing` link. The full sent /
 * never-sent lists moved out of this step with the redraw - the sentence
 * carries the never-shared claim, and the itemised lists stay one click away
 * under Settings ("What is collected").
 *
 * The switch is **on by default and the primary finishes, not "Accept"** -
 * leaving it alone is a real answer. Skipping is also an answer: it records
 * sharing off, because a skipped consent is not consent.
 */
export function DiagnosticsPane({
  share,
  onToggleShare,
  busy,
  onContinue,
  onSkip,
}: {
  share: boolean;
  onToggleShare: () => void;
  busy?: boolean;
  onContinue: () => void;
  /** The drawn "Skip data sharing" link: records sharing off and finishes. */
  onSkip: () => void;
}) {
  return (
    <SetupBody gap={5}>
      <SetupHeader
        mark={<Icon name="share2" size={24} />}
        title="Share diagnostic data"
        subtitle={
          <>
            Opt-in to send Gate errors and routing stats to help fix problems.{" "}
            {/* Medium, and the frame sets it apart on purpose: it is the
             * sentence that says what never leaves the machine. */}
            <span className="font-medium text-base-foreground">
              Prompts, credentials, or private information are never shared.
            </span>
          </>
        }
      />

      <div className="flex items-center gap-3 rounded-md border border-base-border bg-base-card p-3 shadow-base-xs">
        <p className="min-w-0 flex-1 text-sm font-medium leading-5 text-base-foreground">
          Diagnostic data sharing
        </p>
        <span className="flex shrink-0 items-center gap-2">
          <span className="text-sm leading-5 text-base-foreground">
            {share ? "On" : "Off"}
          </span>
          <BaseSwitch on={share} label="Diagnostic data sharing" onClick={onToggleShare} />
        </span>
      </div>

      <div className="flex flex-col gap-2">
        <PrimaryButton onClick={onContinue} busy={busy}>
          Finish setup
        </PrimaryButton>
        <SetupLink onClick={onSkip}>Skip data sharing</SetupLink>
      </div>
    </SetupBody>
  );
}
