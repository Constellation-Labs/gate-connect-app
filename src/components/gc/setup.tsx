import { useId } from "react";
import type { ReactNode } from "react";
import { ConstellationHexMark } from "./ConstellationHexMark";
import { Icon } from "./Icon";
import { ModalOption } from "./Modal";
import { BaseSwitch } from "./base";
import { CollectedDataLists } from "./dialogs";
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
  children,
}: {
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: TopnavAction) => void;
  children: ReactNode;
}) {
  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-base-card">
      <Topbar
        menuOpen={menuOpen}
        onMenuToggle={onMenuToggle}
        onMenuSelect={onMenuSelect}
      />
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto bg-gray-100 p-6">
        <div className="w-full max-w-[520px] rounded-lg border border-base-border bg-base-card p-6 shadow-base-sm">
          {children}
        </div>
      </div>
    </div>
  );
}

function SetupHeader({
  title,
  subtitle,
  mark,
}: {
  title: ReactNode;
  subtitle: string;
  mark?: ReactNode;
}) {
  return (
    <header className="flex flex-col items-center text-center">
      <span className="mb-4">{mark ?? <ConstellationHexMark size={40} />}</span>
      <h1 className="text-xl font-medium leading-6 tracking-heading text-neutral-900">
        {title}
      </h1>
      <p className="mt-1 text-sm leading-5 text-neutral-600">{subtitle}</p>
    </header>
  );
}

function PrimaryButton({
  children,
  onClick,
  busy,
  disabled,
}: {
  children: ReactNode;
  onClick: () => void;
  busy?: boolean;
  /** The Auth panes draw their primary muted until the field below it is filled
   *  and the org list has a selection. Same argument as `ModalButton`. */
  disabled?: boolean;
}) {
  const inert = busy || disabled;
  return (
    <button
      type="button"
      onClick={inert ? undefined : onClick}
      aria-busy={busy || undefined}
      aria-disabled={inert || undefined}
      className={`flex h-9 w-full items-center justify-center rounded-base bg-blue-ribbon-700 px-4 text-sm font-medium text-white transition-colors hover:bg-blue-ribbon-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        inert ? "opacity-70" : ""
      } ${disabled && !busy ? "cursor-not-allowed" : ""}`}
    >
      {children}
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
    <div className="flex items-center gap-3">
      <span aria-hidden className="h-px flex-1 bg-base-border" />
      <span className="text-base-xs leading-4 text-base-muted-foreground">or</span>
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
      className="flex h-9 w-full items-center justify-center rounded-base border border-base-border bg-base-card px-4 text-sm font-medium text-neutral-900 shadow-base-2xs transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
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
}: {
  label: string;
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  mono?: boolean;
  type?: "text" | "password";
}) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-base-xs font-medium leading-4 text-neutral-900">
        {label}
      </label>
      <input
        id={id}
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className={`h-9 w-full rounded-base border border-base-input bg-base-card px-3 text-sm text-neutral-900 shadow-base-2xs placeholder:text-base-muted-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
          mono ? "font-mono" : ""
        }`}
      />
    </div>
  );
}

/** Shown for a failure the user can act on. Mirrors the popover's `ErrorNote`. */
function SetupError({ children }: { children: ReactNode }) {
  return (
    <p
      role="alert"
      className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm leading-5 text-red-900"
    >
      {children}
    </p>
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
          className="-mx-1.5 -my-1.5 px-1.5 py-1.5 text-base-xs font-medium text-neutral-600 underline decoration-base-input underline-offset-2 transition-colors hover:text-neutral-900"
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
          className="text-base-xs font-medium text-neutral-600 transition-colors hover:text-neutral-900"
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
    <div className="flex flex-col gap-6">
      <SetupHeader
        title={
          reauth ? (
            "Session expired"
          ) : (
            <>
              <span className="text-blue-ribbon-800">Gate</span> Connect
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

      <div className="flex flex-col gap-3">
        <PrimaryButton onClick={onSignIn} busy={busy}>
          {reauth ? "Sign in again" : "Continue with Gate account"}
        </PrimaryButton>
        <OrDivider />
        <SecondaryButton onClick={onUseApiKey}>Use an API key</SecondaryButton>
      </div>

      {gateway}
    </div>
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
    <div className="flex flex-col gap-6">
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
        <PrimaryButton onClick={onConnect} busy={busy} disabled={!apiKey.trim()}>
          Connect and continue
        </PrimaryButton>
        <SetupLink onClick={onGoBack}>Go back</SetupLink>
      </div>

      {gateway}
    </div>
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
    <div className="flex flex-col gap-6">
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
        />
        <PrimaryButton onClick={onContinue} busy={busy} disabled={!value.trim()}>
          Continue
        </PrimaryButton>
        <SetupLink onClick={onSkip}>Skip naming</SetupLink>
      </div>
    </div>
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
    <div className="flex flex-col gap-6">
      <SetupHeader
        mark={<Icon name="usersRound" size={24} />}
        title="Choose an organization"
        subtitle="Gate Connect will use the selected organization for routing, activity, and PAYG credits on this device."
      />

      {error && <SetupError>{error}</SetupError>}

      {organizations.length === 0 ? (
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1 rounded-lg border border-amber-300 bg-amber-50 p-3 text-center">
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
            <PrimaryButton onClick={onContinue} busy={busy} disabled={!selectedId}>
              Continue
            </PrimaryButton>
            <SetupLink onClick={onUseDifferentAccount}>Use a different account</SetupLink>
          </div>
        </>
      )}
    </div>
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
    <div className="flex flex-col gap-6">
      <SetupHeader
        title="You're connected"
        subtitle={`Gate Connect is signed in to ${workspace}.`}
        mark={
          <span className="flex size-12 items-center justify-center rounded-lg bg-green-100 text-green-700">
            <Icon name="circleCheck" size={24} />
          </span>
        }
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
    </div>
  );
}

/**
 * The last step before Overview: the diagnostic-data choice.
 *
 * Placed here rather than in Settings-only because consent belongs before
 * collection, not after it. `lib/analytics.ts` starts PostHog at launch, so the
 * first thing this step buys is a person who has actually been asked.
 *
 * **Provisional layout.** The Figma draws no diagnostics step (AG-551 and AG-553
 * are still moving). Structure comes from AG-603 and AG-554: the switch defaults
 * On, the list of what is and is not collected sits under it, and Continue records
 * the displayed value.
 *
 * The switch is **on by default and the primary is Continue, not Accept** -
 * leaving it alone is a real answer, and the copy says so rather than implying the
 * person has to agree to proceed.
 */
export function DiagnosticsPane({
  share,
  onToggleShare,
  busy,
  onContinue,
}: {
  share: boolean;
  onToggleShare: () => void;
  busy?: boolean;
  onContinue: () => void;
}) {
  return (
    <div className="flex flex-col gap-6">
      <SetupHeader
        title="Help fix problems"
        subtitle="Gate Connect can send diagnostic data about itself. You can change this any time in Settings."
        mark={
          <span className="flex size-12 items-center justify-center rounded-lg bg-gray-100 text-neutral-700">
            <Icon name="info" size={24} />
          </span>
        }
      />

      <div className="flex items-center gap-3 rounded-lg border border-base-border bg-base-card p-4">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium leading-5 text-neutral-900">
            Share diagnostic data
          </p>
          <p className="text-base-xs leading-4 text-neutral-600">
            {share
              ? "Gate will send the data listed below."
              : "Gate will not send diagnostic data."}
          </p>
        </div>
        <span className="flex shrink-0 items-center gap-2">
          <span className="text-base-xs font-medium text-neutral-600">
            {share ? "On" : "Off"}
          </span>
          <BaseSwitch on={share} label="Share diagnostic data" onClick={onToggleShare} />
        </span>
      </div>

      <div className="flex flex-col gap-3 text-base-xs leading-4 text-neutral-600">
        <CollectedDataLists Wrapper={SetupNote} />
      </div>

      <PrimaryButton onClick={onContinue} busy={busy}>
        Continue
      </PrimaryButton>
    </div>
  );
}

/** The card the setup pane frames each list in - `ModalNote`'s counterpart here. */
function SetupNote({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg border border-base-border bg-gray-50 p-3">{children}</div>
  );
}
