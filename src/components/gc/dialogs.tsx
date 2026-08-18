import { useRef } from "react";
import type { ReactNode } from "react";
import { Icon } from "./Icon";
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
  onSelect,
  onCancel,
  onConfirm,
}: {
  organizations: DialogOrganization[];
  selectedId: string;
  onSelect: (id: string) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal
      icon="usersRound"
      title="Switch organization"
      subtitle="Choose where this device sends activity and uses Gate credits"
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{ label: "Switch organization", onClick: onConfirm }}
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
      subtitle={`Gate Connect is now using ${organizationName}.`}
      primary={{ label: "Done", onClick: onDone }}
      onDismiss={onDone}
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
  onKeep,
  onReplace,
}: {
  app: DialogApp;
  existingConfig: string;
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
      title="Apply changes to running apps"
      subtitle="Your configuration is now saved. One final step makes the new route active"
      secondary={{ label: "Close affected apps", onClick: onCloseApps }}
      primary={{ label: "I will reopen later", onClick: onReopenLater }}
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
      secondary={{ label: "Go back", onClick: onGoBack }}
      primary={{ label: `Close ${label}`, onClick: onCloseApps, destructive: true }}
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
      title="Diagnostics"
      subtitle="The state of this install, as text you can hand to someone else"
      secondary={{ label: "Close", onClick: onClose }}
      primary={{ label: copied ? "Copied" : "Copy report", onClick: onCopy }}
      onDismiss={onClose}
    >
      <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-lg border border-base-border bg-gray-50 p-4 font-mono text-base-xs leading-4 text-neutral-700">
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
 * "App w/ choose model modal open" frame).
 *
 * The design draws this as a **dropdown anchored to the Change model button**,
 * not a centred dialog: a white rounded panel, one row per model, the current
 * one first and outlined. Rendered here as a modal-positioned popover so it
 * keeps the focus trap and escape handling every other overlay has - anchoring
 * it to the button would need the pane to own the trigger's geometry, and the
 * design's own placement is only legible at one zoom level.
 *
 * Model ids are mono. They are identifiers, and CLAUDE.md names them
 * explicitly; the frame renders them in the UI face, which reads as a slip
 * rather than a decision, since every other identifier in the design is mono.
 *
 * The top edge of the drawn panel was cut off in the only readable capture, so
 * whether it carries a search field is unknown. Omitted here: eleven rows do not
 * need one, and inventing a control the designer may not have drawn is worse
 * than leaving room for it.
 */
export function ModelPickerDialog({
  models,
  selectedId,
  onSelect,
  onDismiss,
}: {
  models: GateModelOption[];
  selectedId?: string;
  onSelect: (id: string) => void;
  onDismiss: () => void;
}) {
  return (
    <Modal
      icon="layers"
      title="Choose a Gate model"
      subtitle="Requests routed through Gate will use this model"
      secondary={{ label: "Cancel", onClick: onDismiss }}
      onDismiss={onDismiss}
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
        <div role="radiogroup" aria-label="Gate model" className="flex flex-col gap-1">
          {models.map((model) => {
            const selected = model.id === selectedId;
            return (
              <button
                key={model.id}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => onSelect(model.id)}
                className={`flex items-center gap-3 rounded-base border px-3 py-2 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
                  selected
                    ? "border-base-primary bg-base-card"
                    : "border-transparent hover:bg-gray-50"
                }`}
              >
                <span aria-hidden className="flex size-4 shrink-0 items-center justify-center">
                  {model.logo ?? <Icon name="cube" size={16} />}
                </span>
                <span className="font-mono text-sm leading-5 text-neutral-900">{model.id}</span>
              </button>
            );
          })}
        </div>
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
      <div className="flex items-center gap-3 rounded-lg border border-base-border p-3">
        <span
          aria-hidden
          className="flex size-10 shrink-0 items-center justify-center rounded-base border border-base-border text-neutral-700"
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
