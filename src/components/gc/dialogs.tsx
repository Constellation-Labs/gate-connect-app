import { useRef } from "react";
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
  /** What Gate would write in its place - the loopback relay this tool's config
   * would be pointed at. Absent when no relay port has been bound yet, in which
   * case the row is omitted rather than guessed at. */
  gateRoute,
  onKeep,
  onReplace,
}: {
  app: DialogApp;
  existingConfig: string;
  gateRoute?: string | null;
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
      subtitle="Only while Share diagnostic data is on. Nothing here identifies you."
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
