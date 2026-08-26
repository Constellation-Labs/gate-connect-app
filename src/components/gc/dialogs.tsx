import { useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Icon } from "./Icon";
import type { RestoreJournal, RestoreOutcome } from "../../lib/api";
import type { PillTone } from "./Modal";
import {
  Modal,
  ModalCheckbox,
  ModalField,
  ModalNote,
  ModalOption,
  ModalSteps,
  ModalSubject,
} from "./Modal";

/**
 * The concrete dialogs, each a thin composition of `Modal`. Copy lives here
 * rather than in the shell so it stays next to the design it came from and the
 * shell only supplies names and handlers.
 *
 * Routing flows: switch organization, organization switched, review config,
 * apply changes, close affected apps, change ready, use a Gate model.
 * Settings flows: rename device, replace API key, disconnect Gate, reset, and
 * the diagnostics report.
 *
 * Presentational throughout: nothing here talks to `lib/api`.
 */

export interface DialogApp {
  name: string;
  /** 16px product mark. Falls back to a cube while the marks are unexported. */
  icon?: ReactNode;
}

/** "these apps" reads wrong for one app and "Codex" reads wrong for three. */
function appLabel(apps: DialogApp[]): string {
  return apps.length === 1 ? apps[0].name : "these apps";
}

function appIcon(app: DialogApp): ReactNode {
  return app.icon ?? <Icon name="cube" size={16} />;
}

export interface DialogOrganization {
  id: string;
  name: string;
  /** Two-letter avatar, e.g. "AE". */
  initials: string;
  /** Secondary line, e.g. "12 members - Free plan". */
  meta: string;
}

export function SwitchOrganizationDialog({
  organizations,
  selectedId,
  currentId,
  onSelect,
  onCancel,
  onConfirm,
}: {
  organizations: DialogOrganization[];
  selectedId: string;
  /** The org this device already uses. While it is the one selected the primary
   * is refused - the drawn dialog mutes it - because confirming a no-op switch
   * would fire the whole switch sequence to change nothing. */
  currentId?: string;
  onSelect: (id: string) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal
      icon="usersRound"
      title="Switch organization"
      subtitle="Select where this device sends activity and uses Gate credits"
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Switch organization",
        onClick: onConfirm,
        disabled: currentId !== undefined && selectedId === currentId,
      }}
      onDismiss={onCancel}
    >
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
    </Modal>
  );
}

export interface DialogGatewayServer {
  label: string;
  url: string;
}

/**
 * The environment switch, for people working on Gate itself.
 *
 * Confirmed rather than applied on the click, and it spells out the three
 * consequences: `switch_gateway` forgets the stored key, disconnects managed
 * tools and stops the engine, which pins the gateway URL when it starts, so the
 * app relaunches into a clean session. Destructive weighting for that reason -
 * the popover's version is a ConfirmPanel with the same sentence.
 */
export function SwitchGatewayDialog({
  servers,
  selectedUrl,
  currentUrl,
  busy,
  onSelect,
  onCancel,
  onConfirm,
}: {
  servers: DialogGatewayServer[];
  selectedUrl: string;
  /** The account's current server, so the dialog can refuse a no-op switch. */
  currentUrl: string;
  busy?: boolean;
  onSelect: (url: string) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal
      tone="warning"
      icon="globe"
      title="Change gateway server"
      subtitle="Point this device at another Gate environment."
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: busy ? "Switching..." : "Switch and relaunch",
        onClick: onConfirm,
        destructive: true,
        disabled: busy || selectedUrl === currentUrl,
      }}
      onDismiss={onCancel}
    >
      <div role="radiogroup" aria-label="Gateway server" className="flex flex-col gap-3">
        {servers.map((server) => (
          <ModalOption
            key={server.url}
            initials={server.label.slice(0, 2).toUpperCase()}
            name={server.label}
            meta={server.url}
            selected={server.url === selectedUrl}
            onSelect={() => onSelect(server.url)}
          />
        ))}
      </div>
      <ModalNote>
        <p className="font-medium text-neutral-900">Switching starts a fresh session.</p>
        <p className="mt-1">
          Your stored key is forgotten, managed tools disconnect, and Gate Connect
          relaunches against the new server.
        </p>
      </ModalNote>
    </Modal>
  );
}

/**
 * The one-time offer to move a pasted key onto Constellation sign-in.
 *
 * Shown once to accounts that predate OAuth or took the key path deliberately;
 * `lib/oauthOffer.ts` remembers the answer whichever way the user leaves. The
 * decline is not a "Not now": a pasted key is a supported choice and the copy
 * says so, which is the popover's `OAuthOffer` argument and its copy.
 *
 * The offer is unsolicited, so nothing destructive-looking is needed to keep
 * focus off the accept: `Modal` opens focus on the first control in the button
 * row, and that is the decline.
 */
export function OAuthOfferDialog({
  secretStore,
  busy,
  error,
  onSignIn,
  onKeepKey,
}: {
  /** "the keychain" / "Credential Manager", named per platform. */
  secretStore: string;
  busy?: boolean;
  error?: ReactNode;
  onSignIn: () => void;
  onKeepKey: () => void;
}) {
  return (
    <Modal
      icon="shieldCheck"
      title="Sign in instead of pasting a key"
      subtitle={`Constellation sign-in keeps your session in ${secretStore} and refreshes it on its own, so there is nothing to rotate when a key expires.`}
      // Guarded rather than `disabled`: `Modal` does not honour that on the
      // secondary, and a decline that lands mid-flow would close the offer over
      // a browser sign-in that is still going to finish.
      secondary={{ label: "Keep using my API key", onClick: () => !busy && onKeepKey() }}
      primary={{
        label: busy ? "Waiting for browser..." : "Sign in with Constellation",
        onClick: onSignIn,
        disabled: busy,
      }}
      onDismiss={busy ? undefined : onKeepKey}
    >
      <p className="text-sm leading-5 text-neutral-600">
        Your gateway and your routing stay exactly as they are. You can switch either
        way later, under Connection in Settings.
      </p>
      {error}
    </Modal>
  );
}

export function OrganizationSwitchedDialog({
  organizationName,
  onDone,
}: {
  organizationName: string;
  onDone: () => void;
}) {
  return (
    <Modal
      tone="success"
      icon="circleCheck"
      title="Organization switched"
      subtitle={`Gate Connect is now using ${organizationName}`}
      primary={{ label: "Done", onClick: onDone }}
      onDismiss={onDone}
      narrow
    >
      <ModalNote>
        <p className="font-medium text-neutral-900">Your local routing is unchanged.</p>
        <p className="mt-1">
          New activity and PAYG usage will appear under {organizationName}.
        </p>
      </ModalNote>
    </Modal>
  );
}

export function ReviewConfigDialog({
  app,
  /** What Gate found, e.g. "API base URL: https://api.openai.com/v1". */
  existingConfig,
  /** What Gate would write in its place - the loopback relay this tool's config
   * would be pointed at. Absent when no relay port has been bound yet, in which
   * case the row is omitted rather than guessed at. */
  gateRoute,
  /** The file that gets rewritten, e.g. "/Users/x/.codex/config.toml".
   *
   * AG-564 asks the warning to name the tool *and* the configuration location.
   * Naming the file is also the transparency this product trades on: the user can
   * go and read it, which is a stronger reassurance than any sentence about what
   * Gate does or does not touch. Omitted when no single file names it. */
  configLocation,
  onKeep,
  onReplace,
}: {
  app: DialogApp;
  existingConfig: string;
  gateRoute?: string | null;
  configLocation?: string | null;
  onKeep: () => void;
  onReplace: () => void;
}) {
  return (
    <Modal
      tone="warning"
      icon="triangleAlert"
      title={`Review ${app.name} configuration`}
      subtitle="Gate found settings that it didn't create. They will not be replaced without your approval"
      secondary={{ label: "Keep existing config", onClick: onKeep }}
      primary={{ label: "Replace config and protect", onClick: onReplace }}
      onDismiss={onKeep}
    >
      <ModalSubject
        icon={appIcon(app)}
        title="Existing custom configuration"
        description={existingConfig}
        pill={{ label: "Detected", tone: "amber" }}
      />
      {/* What replaces it. Approving an overwrite without being shown the
          replacement is approving a value you cannot see - and this is the one
          screen where the user is asked to hand their tool's routing to us. */}
      {gateRoute && (
        <ModalSubject
          icon={appIcon(app)}
          title="What Gate would write instead"
          description={gateRoute}
          pill={{ label: "Gate route", tone: "green" }}
        />
      )}
      {configLocation && (
        <ModalNote>
          <p className="font-medium text-neutral-900">The file that changes:</p>
          {/* Mono, like every other identifier in this UI. `break-all` because a
              home-directory path overflows the 600px dialog on any real machine. */}
          <p className="mt-1 break-all font-mono text-base-xs">{configLocation}</p>
        </ModalNote>
      )}
      <ModalNote>
        <p className="font-medium text-neutral-900">If Gate takes over:</p>
        <p className="mt-1">
          Gate Connect saves a private snapshot of these settings, replaces only the
          routing fields, and keeps the credential in your operating system keychain.
        </p>
        <p className="mt-3">
          Your configuration is restored when you turn protection off, disconnect Gate
          Connect, or do a complete reset.
        </p>
      </ModalNote>
    </Modal>
  );
}

/**
 * Note the button weighting: the design makes "I will reopen later" the filled
 * primary and "Close affected apps" the outline secondary, which is the reverse
 * of the usual arrangement. Deliberate - the quiet option is the safe one here.
 */
export function ApplyChangesDialog({
  apps,
  onCloseApps,
  onReopenLater,
}: {
  apps: DialogApp[];
  onCloseApps: () => void;
  onReopenLater: () => void;
}) {
  return (
    <Modal
      tone="warning"
      icon="triangleAlert"
      title="Apply changes to running apps?"
      subtitle="Your configuration is saved. One final step makes the new route active"
      secondary={{ label: "Yes, close affected apps", onClick: onCloseApps }}
      primary={{ label: "No, I will reopen later", onClick: onReopenLater }}
      onDismiss={onReopenLater}
    >
      {apps.map((app) => (
        <ModalSubject
          key={app.name}
          icon={appIcon(app)}
          title={app.name}
          description="Running now. It will keep its current route until closed."
          pill={{ label: "Open", tone: "green" }}
        />
      ))}
      <ModalNote>
        <p>Gate Connect can close these apps, but cannot reopen them.</p>
        <p className="mt-1">You can keep working and reopen {appLabel(apps)} yourself.</p>
      </ModalNote>
    </Modal>
  );
}

export function CloseAppsDialog({
  apps,
  onGoBack,
  onCloseApps,
}: {
  apps: DialogApp[];
  onGoBack: () => void;
  onCloseApps: () => void;
}) {
  const label = appLabel(apps);
  return (
    <Modal
      tone="warning"
      icon="triangleAlert"
      title="Close affected apps now?"
      subtitle="Unsaved work or active sessions in these apps may be interrupted"
      secondary={{ label: "No, I will close later", onClick: onGoBack }}
      primary={{ label: "Yes, close apps", onClick: onCloseApps, destructive: true }}
      onDismiss={onGoBack}
    >
      {apps.map((app) => (
        <ModalSubject
          key={app.name}
          icon={appIcon(app)}
          title={app.name}
          description="Running now. It will keep its current route until closed."
          pill={{ label: "Open", tone: "green" }}
        />
      ))}
      <ModalNote>
        After these apps are closed, open {label} again yourself. The new Gate route will
        be active on launch.
      </ModalNote>
    </Modal>
  );
}

export function ChangeReadyDialog({
  app,
  onDone,
}: {
  app: DialogApp;
  onDone: () => void;
}) {
  return (
    <Modal
      tone="success"
      icon="circleCheck"
      title="Change is ready"
      subtitle={`${app.name} closed successfully`}
      primary={{ label: "Done", onClick: onDone }}
      onDismiss={onDone}
      narrow
    >
      <ModalNote>
        <p className="font-medium text-neutral-900">The new Gate route is active</p>
        <p className="mt-1">Open {app.name} whenever you are ready to continue.</p>
      </ModalNote>
    </Modal>
  );
}

/**
 * The whole state of this install as text. Shown before it is copied, never
 * copied blind - `screens/Diagnostics.tsx` argues the point and it still holds:
 * this app installs a root certificate, runs a local proxy and holds a
 * credential, so a button that silently loads the clipboard with an unseen
 * description of that setup is the opposite of the reassurance it is meant to
 * provide.
 */
export function DiagnosticsDialog({
  report,
  copied,
  onCopy,
  onClose,
}: {
  /** Pre-built by `lib/diagnosticsReport`. */
  report: string;
  /** Flips the primary button's label after a successful copy. */
  copied?: boolean;
  onCopy: () => void;
  onClose: () => void;
}) {
  return (
    <Modal
      icon="info"
      title="Diagnostics report"
      // The drawn subtitle reads "this installed" - a typo, kept corrected.
      subtitle="The state of this install, as text you can hand to someone else"
      secondary={{ label: "Close", onClick: onClose }}
      primary={{ label: copied ? "Copied" : "Copy report", onClick: onCopy }}
      onDismiss={onClose}
    >
      <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-base-border bg-gray-50 p-4 font-mono text-base-xs leading-4 text-neutral-700">
        {report}
      </pre>
    </Modal>
  );
}

/** One selectable Gate model. */
export interface GateModelOption {
  /** Fully qualified id, e.g. "gate/opus 5". Rendered mono - it is an identifier. */
  id: string;
  /** Who makes the model, for the glyph and for grouping in the caller's head. */
  vendor: string;
  /** 16px vendor mark. Falls back to a cube while the marks are unexported. */
  logo?: ReactNode;
}

/**
 * Choosing which Gate model an app runs on (Figma `App / Select a model`, the
 * "App w/ choose model modal open" frame, re-read 2026-08-21).
 *
 * The design now draws this as a centred dialog rather than the dropdown an
 * earlier read described: header with an X, a search field beside an
 * "All providers" filter, a "Showing X of Y models" line, then one radio row
 * per model with the selected one outlined. One deliberate deviation:
 *
 * - The drawn subtitle reads "<App> uses on Gate model", which is not a
 *   sentence; rendered with the one word that makes it one. Raised as a copy
 *   question rather than shipped verbatim.
 *
 * Model ids are mono. They are identifiers, and CLAUDE.md names them
 * explicitly; the frame renders them in the UI face, which reads as a slip
 * rather than a decision, since every other identifier in the design is mono.
 */
export function ModelPickerDialog({
  appName,
  models,
  selectedId,
  onSelect,
  onDismiss,
}: {
  /** Whose model is being chosen, for the subtitle. */
  appName: string;
  models: GateModelOption[];
  selectedId?: string;
  onSelect: (id: string) => void;
  onDismiss: () => void;
}) {
  const [query, setQuery] = useState("");
  const [vendor, setVendor] = useState("all");
  const searchRef = useRef<HTMLInputElement>(null);

  const vendors = useMemo(
    () => [...new Set(models.map((m) => m.vendor))].sort((a, b) => a.localeCompare(b)),
    [models],
  );
  const needle = query.trim().toLowerCase();
  // "Current models will sort alphabetically, left to right using their
  // provider. Example. Anthropic > DeepSeek > Moonshot" - written on the
  // `App / Select multiple models (Opencode)` section, read 2026-08-26. By
  // provider first, then by id so a provider's own models hold a stable order
  // rather than falling back to whatever the gateway listed.
  const shown = useMemo(
    () =>
      models
        .filter(
          (m) =>
            (vendor === "all" || m.vendor === vendor) &&
            m.id.toLowerCase().includes(needle),
        )
        .sort((a, b) => a.vendor.localeCompare(b.vendor) || a.id.localeCompare(b.id)),
    [models, vendor, needle],
  );

  return (
    <Modal
      icon="layers"
      title="Choose a Gate model"
      subtitle={`What ${appName} uses on Gate model`}
      onClose={onDismiss}
      onDismiss={onDismiss}
      initialFocus={searchRef}
    >
      {models.length === 0 ? (
        <ModalNote>
          <p className="font-medium text-neutral-900">No models to choose from yet</p>
          <p className="mt-1">
            Gate will list the models your gateway offers here. Until then, apps keep
            using the model they are configured with.
          </p>
        </ModalNote>
      ) : (
        <>
          <div className="flex items-center gap-3">
            <label className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-sm border border-base-input bg-base-card px-2.5 shadow-base-2xs focus-within:outline focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-base-primary">
              <Icon name="search" size={16} className="shrink-0 text-neutral-500" />
              <input
                ref={searchRef}
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search models"
                aria-label="Search models"
                className="w-full bg-transparent text-sm leading-5 text-neutral-900 outline-none placeholder:text-neutral-500"
              />
            </label>
            <select
              value={vendor}
              onChange={(e) => setVendor(e.target.value)}
              aria-label="Provider"
              className="h-9 shrink-0 rounded-sm border border-base-input bg-base-card px-2.5 text-sm font-medium leading-5 text-neutral-900 shadow-base-2xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
            >
              <option value="all">All providers</option>
              {vendors.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </select>
          </div>

          {/* The line stays true when it reads "Showing 0 of 14": a search with
            * no hits already says so in the one place the user is looking. */}
          <p className="text-base-xs leading-4 text-base-muted-foreground">
            Showing {shown.length} of {models.length} models · 400+ in Gate AI
          </p>

          <div
            role="radiogroup"
            aria-label="Gate model"
            className="-mr-2 flex max-h-80 flex-col gap-1 overflow-y-auto pr-2"
          >
            {shown.map((model) => {
              const selected = model.id === selectedId;
              return (
                <button
                  key={model.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => onSelect(model.id)}
                  className={`flex items-center gap-3 rounded-sm border px-3 py-2 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
                    selected
                      ? "border-base-primary bg-base-card"
                      : "border-base-border hover:bg-gray-50"
                  }`}
                >
                  <span aria-hidden className="flex size-4 shrink-0 items-center justify-center">
                    {model.logo ?? <Icon name="cube" size={16} />}
                  </span>
                  <span className="min-w-0 flex-1 truncate font-mono text-sm leading-5 text-neutral-900">
                    {model.id}
                  </span>
                  {selected ? (
                    <Icon
                      name="circleCheck"
                      size={16}
                      className="shrink-0 text-base-primary"
                    />
                  ) : (
                    <span
                      aria-hidden
                      className="size-4 shrink-0 rounded-full border border-base-input"
                    />
                  )}
                </button>
              );
            })}
          </div>
        </>
      )}
    </Modal>
  );
}

export function UseGateModelDialog({
  app,
  vendor,
  modelId,
  /** Pre-formatted balance, e.g. "$10.25 available". */
  credits,
  vendorLogo,
  onKeepAppDefault,
  onUseGateCredits,
}: {
  app: DialogApp;
  vendor: string;
  modelId: string;
  credits: string;
  vendorLogo?: ReactNode;
  onKeepAppDefault: () => void;
  onUseGateCredits: () => void;
}) {
  return (
    <Modal
      icon="layers"
      title={`Use a Gate model for ${app.name}?`}
      subtitle="Your next requests will use Constellation Gate PAYG credits"
      subtitleTone="primary"
      secondary={{ label: "Keep App default", onClick: onKeepAppDefault }}
      primary={{ label: "Use Gate credits", onClick: onUseGateCredits }}
      onDismiss={onKeepAppDefault}
      narrow
    >
      <ModalSubject
        icon={vendorLogo ?? <Icon name="cube" size={16} />}
        title={vendor}
        description={modelId}
        variant="identity"
        pill={{ label: "PAYG", tone: "neutral" }}
      />

      {/* Label left, balance right - the one row in the dialogs that reads
       * across rather than stacking, so it is not a `ModalSubject`. */}
      <div className="flex items-center gap-3 rounded-md border border-base-border p-3">
        <span
          aria-hidden
          className="flex size-10 shrink-0 items-center justify-center rounded-sm border border-base-border text-neutral-700"
        >
          <Icon name="creditCard" size={16} />
        </span>
        <p className="flex-1 text-sm leading-5 text-neutral-600">Gate credits:</p>
        <p className="shrink-0 text-sm font-medium leading-5 text-neutral-900">
          {credits}
        </p>
      </div>
      <ModalNote>
        {app.name}'s own model preference is not changed. You can return to App default at
        any time.
      </ModalNote>
    </Modal>
  );
}

/**
 * Rename the device. Shows the current name read-only above the new one so the
 * user can see what they are replacing rather than trusting the label. Focus
 * opens on the editable field, not the read-only one above it.
 */
export function RenameDeviceDialog({
  currentName,
  newName,
  onNewNameChange,
  onCancel,
  onRename,
}: {
  currentName: string;
  newName: string;
  onNewNameChange: (next: string) => void;
  onCancel: () => void;
  onRename: () => void;
}) {
  const field = useRef<HTMLInputElement>(null);
  return (
    <Modal
      icon="monitorSmartphone"
      title="Rename your device"
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{ label: "Rename device", onClick: onRename, disabled: !newName.trim() }}
      onDismiss={onCancel}
      initialFocus={field}
    >
      <ModalField label="Current device name" value={currentName} readOnly />
      <ModalField
        label="New device name"
        value={newName}
        onChange={onNewNameChange}
        inputRef={field}
      />
    </Modal>
  );
}

/**
 * Replace the API key.
 *
 * The design labels the second field "New device name", copy-pasted from the
 * rename dialog. Implemented as "New API key" deliberately: shipping the drawn
 * label would put a wrong word on the one screen where the user is handling a
 * credential. Raised with the designer.
 */
export function ReplaceApiKeyDialog({
  currentKeyMasked,
  newKey,
  onNewKeyChange,
  onCancel,
  onReplace,
}: {
  currentKeyMasked: string;
  newKey: string;
  onNewKeyChange: (next: string) => void;
  onCancel: () => void;
  onReplace: () => void;
}) {
  const field = useRef<HTMLInputElement>(null);
  return (
    <Modal
      icon="key"
      title="Replace API key"
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{ label: "Replace key", onClick: onReplace, disabled: !newKey.trim() }}
      onDismiss={onCancel}
      initialFocus={field}
    >
      <ModalField label="Current API key" value={currentKeyMasked} readOnly mono />
      <ModalField
        label="New API key"
        value={newKey}
        onChange={onNewKeyChange}
        mono
        placeholder="sk-gw..."
        inputRef={field}
      />
    </Modal>
  );
}

/**
 * End the signed-in session. Red tone and a primary that names what it does,
 * because it stops this device talking to Gate.
 *
 * The drawn copy says protection turns off, apps stop routing and the API key is
 * removed from the keychain. That describes Reset, which is a separate row on the
 * same screen; this one sits under "Active session" and ends the session, leaving
 * the account and the tools' configs alone. Copy corrected to match what it does
 * rather than shipping two destructive actions that claim the same consequences.
 * Raised with the designer.
 */
export function DisconnectGateDialog({
  onCancel,
  onDisconnect,
}: {
  onCancel: () => void;
  onDisconnect: () => void;
}) {
  return (
    <Modal
      tone="danger"
      icon="triangleAlert"
      title="Disconnect Gate?"
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Yes, disconnect Gate",
        onClick: onDisconnect,
        destructive: true,
      }}
      onDismiss={onCancel}
    >
      <p className="text-sm leading-5 text-neutral-600">
        This device signs out of Gate and stops sending activity. Your apps keep their
        current configuration, and signing back in restores routing.
      </p>
    </Modal>
  );
}

/**
 * The most destructive action in the app, and the only one gated by an
 * acknowledgement. It spells out its three consequences rather than asserting
 * they exist, and the primary stays refused until the checkbox is ticked.
 */
export function ResetGateConnectDialog({
  acknowledged,
  onAcknowledgedChange,
  onCancel,
  onReset,
}: {
  acknowledged: boolean;
  onAcknowledgedChange: (next: boolean) => void;
  onCancel: () => void;
  onReset: () => void;
}) {
  return (
    <Modal
      tone="danger"
      icon="triangleAlert"
      title="Reset Gate Connect"
      subtitle="This removes Gate Connect setup from this device."
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Reset Gate Connect",
        onClick: onReset,
        destructive: true,
        disabled: !acknowledged,
      }}
      onDismiss={onCancel}
    >
      <ModalSteps
        label="What happens next:"
        steps={[
          {
            title: "Routing turns off",
            description: "Managed tools return to their saved pre_gate configurations.",
          },
          {
            title: "Tools disconnect",
            description: "No app on this device remains protected by Gate.",
          },
          {
            title: "Account and keys are removed",
            description:
              "Your local sign-in, organization, and keychain credentials are cleared.",
          },
        ]}
      />
      <ModalCheckbox
        checked={acknowledged}
        onChange={onAcknowledgedChange}
        label="I understand that setup will restart on this device"
      />
    </Modal>
  );
}

/**
 * What the diagnostic channel actually sends, and what it never sends.
 *
 * AG-603 asks for a "What is collected" list that opens without changing the
 * setting - so this is read-only and its only action closes it.
 *
 * The lists are written from `lib/analytics.ts` rather than from the ticket. The
 * ticket enumerates fields for an upload that does not exist yet (installation
 * name, verification state, event-delivery state, notification permission); the
 * channel that *does* exist is PostHog, sending a closed set of event names, a
 * filtered prop allowlist, classified error titles, and two coarse
 * super-properties. Describing the ticket's list would be describing something
 * Gate does not do, on the one screen whose whole job is telling the truth about
 * what leaves the machine.
 */
export function CollectedDataDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal
      tone="neutral"
      icon="info"
      title="What Gate Connect collects"
      // The first list is gated on the diagnostics toggle; the second is not, and
      // saying "only while it is on" over both would understate it.
      subtitle="Nothing here identifies you. Most of it is sent only while Share diagnostic data is on."
      primary={{ label: "Close", onClick: onClose }}
      onDismiss={onClose}
    >
      <CollectedDataLists Wrapper={ModalNote} />
    </Modal>
  );
}

/**
 * The sent / never-sent lists, shared by the Settings dialog above and the
 * onboarding step in `setup.tsx`.
 *
 * One copy on purpose. Two would drift, and these are the claims the product's
 * reassurance rests on - the moment the onboarding promise and the Settings
 * disclosure disagree, neither can be trusted.
 *
 * `Wrapper` because the two callers frame it differently: the dialog uses
 * `ModalNote`, the setup pane its own card. The content is what is shared, not the
 * chrome.
 */
export function CollectedDataLists({
  Wrapper,
}: {
  Wrapper: (props: { children: ReactNode }) => ReactNode;
}) {
  return (
    <>
      <Wrapper>
        <p className="font-medium text-neutral-900">Sent</p>
        <ul className="mt-1 list-disc pl-4">
          <li>
            An anonymous device id, generated locally. No name, email, or account
            identifier.
          </li>
          <li>App version and operating system.</li>
          <li>
            Which action happened, from a fixed list - routing turned on or off, an
            update installed, a dialog shown. Never free text.
          </li>
          <li>
            A short label for each action: which app or provider it concerned, and
            whether it was on or off.
          </li>
          <li>
            A classified title when something fails, e.g. &ldquo;keychain
            denied&rdquo;. The underlying message stays on this machine.
          </li>
        </ul>
      </Wrapper>
      {/* A third list, and deliberately not folded into the first. Everything
          above is gated on the diagnostics toggle; these two headers ride every
          routed request whatever that toggle says, because they are routing
          metadata rather than telemetry. Leaving them out of this dialog while
          the app started sending them would make the page that exists to be
          trusted the one place that understated what leaves the machine. */}
      <Wrapper>
        <p className="font-medium text-neutral-900">Sent with your traffic, whatever this setting says</p>
        <ul className="mt-1 list-disc pl-4">
          <li>
            The same anonymous device id, so your activity view can group requests
            by machine. It identifies nothing else and authorizes nothing.
          </li>
          <li>
            Which app made the request, when Gate can tell from the request itself
            - Claude Code, Codex, and so on. Unrecognised apps are sent unlabelled
            rather than guessed at.
          </li>
        </ul>
      </Wrapper>
      <Wrapper>
        <p className="font-medium text-neutral-900">Never sent</p>
        <ul className="mt-1 list-disc pl-4">
          <li>Prompts or model responses.</li>
          <li>API keys, credentials, or anything from your keychain.</li>
          <li>File paths, hostnames, or the contents of any config file.</li>
          <li>The text of an error, as opposed to its classification.</li>
        </ul>
      </Wrapper>
    </>
  );
}

/** What each outcome means, in the user's words rather than the enum's. */
const RESTORE_OUTCOME_TEXT: Record<
  RestoreOutcome,
  { label: string; detail: string; tone: PillTone }
> = {
  pending: {
    label: "Not reached",
    detail: "Gate stopped before getting to this one.",
    tone: "amber",
  },
  restored: {
    label: "Done",
    detail: "Configuration written. Whether it is routing is shown on its row.",
    tone: "green",
  },
  write_failed: {
    label: "Failed",
    detail: "Gate could not write the configuration. Resuming tries this one again.",
    tone: "amber",
  },
  not_installed: {
    label: "Skipped",
    detail: "No longer installed, so there was nothing to restore.",
    tone: "neutral",
  },
  unknown: {
    label: "Skipped",
    detail: "Gate does not recognise this one any more.",
    tone: "neutral",
  },
  deferred_signed_out: {
    label: "Waiting",
    detail: "Nothing was attempted: there is no account to point it at yet.",
    tone: "amber",
  },
};

/**
 * What the last restore did, entry by entry.
 *
 * **Read-only, and deliberately so.** AG-570 requires that reviewing details "does
 * not change state" - so the only action closes it, and nothing here can be
 * clicked into a retry. Resuming is the banner's job.
 *
 * There are no credentials, paths or request content in a journal entry: it holds
 * slugs, display names, an outcome from a closed set, and a timestamp. That is what
 * makes it safe to show in full.
 *
 * **Provisional layout.** The Figma draws no details view (AG-569 is To Do).
 */
export function RestoreDetailsDialog({
  journal,
  onClose,
}: {
  journal: RestoreJournal;
  onClose: () => void;
}) {
  const done = journal.entries.filter((e) => e.outcome === "restored").length;
  return (
    <Modal
      tone="neutral"
      icon="info"
      title="What happened to routing"
      subtitle={
        journal.requested_routing_on
          ? `Gate was turning routing back on. ${done} of ${journal.entries.length} finished.`
          : `Gate was turning routing off. ${done} of ${journal.entries.length} finished.`
      }
      primary={{ label: "Close", onClick: onClose }}
      onDismiss={onClose}
    >
      {journal.entries.length === 0 ? (
        <ModalNote>Nothing was recorded for this attempt.</ModalNote>
      ) : (
        journal.entries.map((entry) => {
          const text = RESTORE_OUTCOME_TEXT[entry.outcome];
          return (
            <ModalSubject
              key={`${entry.kind}:${entry.slug}`}
              icon="cube"
              title={entry.name}
              description={text.detail}
              pill={{ label: text.label, tone: text.tone }}
            />
          );
        })
      )}
    </Modal>
  );
}

/** "Claude Code", "Claude Code and Codex", "Claude Code, Codex, and OpenCode". */
function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}

/**
 * Quit, with the three outcomes AG-596 names: disconnect the tools first, quit
 * and leave them pointing at Gate, or don't quit.
 *
 * **Provisional layout.** The Figma draws no quit dialog for the window shell
 * (AG-595 is still To Do), so the structure here comes from the acceptance
 * criteria and the popover's `QuitConfirm`, which already implements the same
 * three choices. The copy is deliberately shared with that panel: a user who has
 * seen one should not have to work out whether the other means something else.
 *
 * Three buttons rather than two because the outcomes genuinely differ. Collapsing
 * "quit without disconnecting" into the primary would hide the consequence that
 * makes it different, and collapsing Cancel would leave no way out.
 *
 * Cancel takes focus, not the primary: the user asked to quit, but Enter on a
 * panel they have not read should not decide *how*.
 */
export function QuitDialog({
  tools,
  busy,
  onDisconnectAndQuit,
  onQuitAnyway,
  onCancel,
}: {
  /** Config-routed tools still pointed at Gate. Never empty - the shell only
   * raises this dialog when the backend reports at least one. */
  tools: string[];
  busy?: boolean;
  onDisconnectAndQuit: () => void;
  onQuitAnyway: () => void;
  onCancel: () => void;
}) {
  const names = joinNames(tools);
  const plural = tools.length > 1;
  return (
    <Modal
      tone="warning"
      icon="logOut"
      title="Quit Gate Connect?"
      secondary={{ label: "Cancel", onClick: onCancel, disabled: busy }}
      middle={{
        label: "Quit without disconnecting",
        onClick: onQuitAnyway,
        disabled: busy,
      }}
      primary={{
        label: busy ? "Working…" : "Disconnect tools and quit",
        onClick: onDisconnectAndQuit,
        disabled: busy,
      }}
      onDismiss={busy ? undefined : onCancel}
    >
      <p className="text-sm leading-5 text-neutral-600">
        {names} still {plural ? "route" : "routes"} through Gate. Quitting stops the
        local relay {plural ? "they" : "it"} points at, so {plural ? "they" : "it"}{" "}
        {plural ? "cannot" : "cannot"} reach a model until Gate Connect runs again.
      </p>
      <p className="text-sm leading-5 text-neutral-600">
        {/* "when Gate Connect starts again", not "at the next start": the next
            start of *what* was the ambiguity, and the tool's own launch is the
            wrong answer. Same phrasing as the notification this fires. */}
        Disconnecting puts {plural ? "their" : "its"} own settings back for the
        meantime, then reconnects {plural ? "them" : "it"} when Gate Connect starts
        again. Routing stays switched on either way.
      </p>
    </Modal>
  );
}

/**
 * What the quit teardown could not finish.
 *
 * AG-596 is explicit that Gate Connect "does not claim cleanup completed", so a
 * partial teardown gets its own dialog rather than a silent exit: the named tools
 * are still pointed at a relay that dies with this process. Retrying is the
 * primary; quitting anyway stays available, because refusing to let someone quit
 * their own app is worse than letting them quit informed.
 */
export function QuitLeftBehindDialog({
  tools,
  busy,
  onRetry,
  onQuitAnyway,
  onCancel,
}: {
  tools: string[];
  busy?: boolean;
  onRetry: () => void;
  onQuitAnyway: () => void;
  onCancel: () => void;
}) {
  const plural = tools.length > 1;
  return (
    <Modal
      tone="warning"
      icon="triangleAlert"
      title={plural ? "Some tools stayed on Gate" : "One tool stayed on Gate"}
      secondary={{ label: "Cancel", onClick: onCancel, disabled: busy }}
      middle={{ label: "Quit anyway", onClick: onQuitAnyway, disabled: busy }}
      primary={{ label: busy ? "Working…" : "Try again", onClick: onRetry, disabled: busy }}
      onDismiss={busy ? undefined : onCancel}
    >
      <p className="text-sm leading-5 text-neutral-600">
        Couldn’t put {joinNames(tools)} back on{" "}
        {plural ? "their own settings" : "its own settings"}.{" "}
        {plural ? "They still point" : "It still points"} at Gate, and won’t reach a
        model until Gate Connect runs again.
      </p>
      <ModalNote>
        Everything else was put back. Trying again only retouches the{" "}
        {plural ? "tools" : "tool"} above.
      </ModalNote>
    </Modal>
  );
}
