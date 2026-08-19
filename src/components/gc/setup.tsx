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
 * **None of these are in the Figma.** The design starts from a signed-in window
 * with apps already listed, so first run had nowhere to land. They are built
 * from the design's own vocabulary - the window chrome, one centred card, the
 * dialog button weighting - rather than invented wholesale, and are explicitly
 * provisional. See plans/new-app-ui-figma.md.
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
  title: string;
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
}: {
  children: ReactNode;
  onClick: () => void;
  busy?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={busy ? undefined : onClick}
      aria-busy={busy || undefined}
      aria-disabled={busy || undefined}
      className={`flex h-9 w-full items-center justify-center rounded-base bg-blue-ribbon-700 px-4 text-sm font-medium text-white transition-colors hover:bg-blue-ribbon-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        busy ? "opacity-70" : ""
      }`}
    >
      {children}
    </button>
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
  apiKeyOpen,
  onToggleApiKey,
  apiKey,
  onApiKeyChange,
  onConnectWithApiKey,
  gateway,
  busy,
  error,
}: {
  reauth?: boolean;
  onSignIn: () => void;
  apiKeyOpen: boolean;
  onToggleApiKey: () => void;
  apiKey: string;
  onApiKeyChange: (next: string) => void;
  onConnectWithApiKey: () => void;
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
        title={reauth ? "Session expired" : "Welcome to Gate Connect"}
        subtitle={
          reauth
            ? "Sign in again to keep routing your apps through Gate."
            : "Point your AI tools at Gate once, and stop thinking about credentials."
        }
      />

      {error && <SetupError>{error}</SetupError>}

      <div className="flex flex-col gap-3">
        <PrimaryButton onClick={onSignIn} busy={busy}>
          {reauth ? "Sign in again" : "Sign in with Constellation"}
        </PrimaryButton>
        <SecondaryButton onClick={onToggleApiKey}>
          {apiKeyOpen ? "Hide API key option" : "Use a Gate API key instead"}
        </SecondaryButton>
      </div>

      {apiKeyOpen && (
        <div className="flex flex-col gap-3 border-t border-base-border pt-6">
          <TextField
            label="Gate API key"
            value={apiKey}
            onChange={onApiKeyChange}
            placeholder="sk-gw..."
            mono
            type="password"
          />
          <PrimaryButton onClick={onConnectWithApiKey} busy={busy}>
            Connect
          </PrimaryButton>
        </div>
      )}

      {gateway}
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
  onUseApiKey,
  busy,
  error,
}: {
  organizations: SetupOrganization[];
  selectedId?: string;
  onSelect: (id: string) => void;
  onContinue: () => void;
  /** The only way forward for an account with no organization. */
  onUseApiKey: () => void;
  busy?: boolean;
  error?: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-6">
      <SetupHeader
        title="Choose an organization"
        subtitle="This decides where your activity is recorded and whose Gate credits you use."
      />

      {error && <SetupError>{error}</SetupError>}

      {organizations.length === 0 ? (
        <div className="flex flex-col gap-3">
          <p className="text-sm leading-5 text-neutral-600">
            This account is not in any organization yet. You can connect with a Gate API
            key instead.
          </p>
          <SecondaryButton onClick={onUseApiKey}>Use a Gate API key</SecondaryButton>
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
          <PrimaryButton onClick={onContinue} busy={busy}>
            Continue
          </PrimaryButton>
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
