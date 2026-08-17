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
