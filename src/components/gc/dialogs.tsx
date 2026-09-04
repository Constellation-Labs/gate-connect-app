import { useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Icon } from "./Icon";
import { Skeleton } from "./base";
import { DEVICE_NAME_MAX_LENGTH } from "../../lib/api";
import type { RecoverySummary, TeardownReport } from "../../lib/api";
import type { RecoveryRow } from "../../lib/recovery";
import {
  operationLine,
  recoveryRows,
  stageCounts,
  TEARDOWN_ACTION_LABEL,
} from "../../lib/recovery";
import type { PillTone } from "./Modal";
import {
  Modal,
  ModalCheckbox,
  ModalChoice,
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
 * Presentational throughout: nothing here calls `lib/api`. The one value it
 * takes from there is `DEVICE_NAME_MAX_LENGTH`, a number the backend owns -
 * a second copy of it here would be a limit that could drift out of step
 * with the one that actually truncates.
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
      width={512}
      subtitle="Select where this device sends activity and uses Gate credits"
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Switch organization",
        onClick: onConfirm,
        disabled: currentId !== undefined && selectedId === currentId,
      }}
      onDismiss={onCancel}
    >
      <div
        role="radiogroup"
        aria-label="Organization"
        className="flex flex-col gap-3"
      >
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
      <div
        role="radiogroup"
        aria-label="Gateway server"
        className="flex flex-col gap-3"
      >
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
        <p className="font-medium text-base-foreground">
          Switching starts a fresh session.
        </p>
        <p className="mt-1">
          Your stored key is forgotten, managed tools disconnect, and Gate
          Connect relaunches against the new server.
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
      secondary={{
        label: "Keep using my API key",
        onClick: () => !busy && onKeepKey(),
      }}
      primary={{
        label: busy ? "Waiting for browser..." : "Sign in with Constellation",
        onClick: onSignIn,
        disabled: busy,
      }}
      onDismiss={busy ? undefined : onKeepKey}
    >
      <p className="text-sm leading-5 text-neutral-600">
        Your gateway and your routing stay exactly as they are. You can switch
        either way later, under Connection in Settings.
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
      width={512}
    >
      <ModalNote>
        <p className="font-medium text-base-foreground">
          Your local routing is unchanged.
        </p>
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
          <p className="font-medium text-base-foreground">
            The file that changes:
          </p>
          {/* Mono, like every other identifier in this UI. `break-all` because a
              home-directory path overflows the 600px dialog on any real machine. */}
          <p className="mt-1 break-all font-mono text-base-xs">
            {configLocation}
          </p>
        </ModalNote>
      )}
      <ModalNote>
        <p className="font-medium text-base-foreground">If Gate takes over:</p>
        <p className="mt-1">
          Gate Connect saves a private snapshot of these settings, replaces only
          the routing fields, and keeps the credential in your operating system
          keychain.
        </p>
        <p className="mt-3">
          Your configuration is restored when you turn protection off,
          disconnect Gate Connect, or do a complete reset.
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
        <p className="mt-1">
          You can keep working and reopen {appLabel(apps)} yourself.
        </p>
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
      primary={{
        label: "Yes, close apps",
        onClick: onCloseApps,
        destructive: true,
      }}
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
        After these apps are closed, open {label} again yourself. The new Gate
        route will be active on launch.
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
      width={512}
    >
      <ModalNote>
        <p className="font-medium text-base-foreground">
          The new Gate route is active
        </p>
        <p className="mt-1">
          Open {app.name} whenever you are ready to continue.
        </p>
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
      // `ClipboardList`, as `363:9030` draws it and as the Settings row that
      // opens this dialog already used. The 2026-08-26 glyph sweep corrected
      // the rows and never followed into the dialogs.
      icon="clipboardList"
      // Neutral, but drawn at the 600px tile (`363:9029`): 44px with a 24px
      // glyph. Tile size follows the width here, not the tone.
      tile="lg"
      title="Diagnostics report"
      // The drawn subtitle reads "this installed" - a typo, kept corrected.
      subtitle="The state of this install, as text you can hand to someone else"
      secondary={{ label: "Close", onClick: onClose }}
      primary={{ label: copied ? "Copied" : "Copy report", onClick: onCopy }}
      onDismiss={onClose}
    >
      {/* `mono/body-14` (`363:9120`), not the 12/16 this rendered: the report is
       * the one screen a user reads a wall of text on. */}
      <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-base-border bg-gray-50 p-4 font-mono text-sm leading-5 text-neutral-700">
        {report}
      </pre>
    </Modal>
  );
}

/** One selectable Gate model. */
export interface GateModelOption {
  /** Canonical id, e.g. `anthropic/claude-opus-5`. Rendered mono - it is an
   *  identifier, which CLAUDE.md names explicitly. The Figma draws these in the
   *  UI face, which reads as a slip rather than a decision since every other
   *  identifier in the design is mono. */
  id: string;
  /** Who makes the model, for the glyph, the provider filter and grouping in the
   *  reader's head. */
  vendor: string;
  /** 16px vendor mark. Falls back to a cube while the marks are unexported. */
  logo?: ReactNode;
}

/**
 * Choosing which Gate model an app runs on (Figma 139:66117, `card/choose-model`).
 *
 * A centred 600px dialog with a search field, a provider filter, a count line and
 * a scrolling list - not the dropdown anchored to the Change model button that an
 * earlier revision of this comment described. That mattered once the catalogue
 * turned out to hold 344 models rather than the eleven the frame draws: a list
 * that long is unusable without search, which is presumably why the design has
 * one.
 *
 * **Model ids stay canonical.** The frame draws a `gate/...` namespace
 * (`gate/opus 5`, `gate/kimi-k3`) which no catalogue serves; the real ids are
 * `provider/model` (`anthropic/claude-opus-5`). Rendering the drawn ids would put
 * a fabricated catalogue in front of the user, which is the same argument the
 * zeroed metrics make.
 *
 * **Multi-select is the design** (AG-589, settled): a checkbox per model, with a
 * footer stating how many are enabled. 139:66117 draws the radios of the
 * single-model era, so a reader diffing against that frame is looking at the
 * older state rather than a divergence - everything around the control still
 * comes from it.
 */
export function ModelPickerDialog({
  appName,
  models,
  loading,
  failure,
  selectedIds,
  onSave,
  onDismiss,
}: {
  /** Named in the subtitle, as the frame does. */
  appName: string;
  models: GateModelOption[];
  /** The catalogue has not landed. Distinct from an empty one, which is a real
   *  answer: a gateway with no platform provider accounts offers nothing, and
   *  saying "no models" while the list is still coming would be a claim we have
   *  not earned. */
  loading?: boolean;
  /** The catalogue could not be read, in the gateway's own words. Distinct again
   *  from empty: "we could not ask" is not "there are none". */
  failure?: string | null;
  /** Already-chosen ids, in the user's order. */
  selectedIds: string[];
  /** The whole set, applied on Save. A set is not a sequence of independent
   *  clicks - AG-590 requires the final model not be removable without choosing
   *  another - so it is confirmed once rather than written per toggle, and
   *  Cancel is a real cancel. */
  onSave: (ids: string[]) => void;
  onDismiss: () => void;
}) {
  const [query, setQuery] = useState("");
  const [vendor, setVendor] = useState("all");
  /** The dialog opens on its search field: with a catalogue this long, typing is
   *  the first thing to do. */
  const searchRef = useRef<HTMLInputElement>(null);

  /** Seeded from the stored set so Cancel is a real cancel. */
  const [draft, setDraft] = useState<string[]>(selectedIds);

  /**
   * Chosen models the catalogue no longer offers (AG-592).
   *
   * They have to be listed, or the set contains something the user cannot reach:
   * a model absent from the catalogue renders no row, so there is no checkbox to
   * clear and no way out except abandoning the whole selection. Shown at the top,
   * marked, and removable - which is the recovery the ticket asks for.
   */
  const missing = useMemo(() => {
    const servable = new Set(models.map((m) => m.id));
    // Derived from the DRAFT, not from what is stored: clearing one has to make
    // the row go, and deriving from the stored set left it on screen still
    // marked enabled while the footer count disagreed. Its absence afterwards is
    // also what satisfies "an unavailable model cannot be selected" - there is no
    // row left to re-check.
    return draft.filter((id) => !servable.has(id));
  }, [models, draft]);

  const vendors = useMemo(
    () =>
      [...new Set(models.map((m) => m.vendor))].sort((a, b) =>
        a.localeCompare(b),
      ),
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
            (needle === "" ||
              m.id.toLowerCase().includes(needle) ||
              m.vendor.toLowerCase().includes(needle)),
        )
        .sort(
          (a, b) =>
            a.vendor.localeCompare(b.vendor) || a.id.localeCompare(b.id),
        ),
    [models, vendor, needle],
  );

  const chosen = draft;
  /** AG-590: the last model cannot be removed without choosing another. The
   *  remedy the ticket names is "or return to Tool default", which is the pane's
   *  radio, not this dialog - so here the last one simply refuses to clear, and
   *  the footer says why. */
  const wouldEmpty = (id: string) => chosen.length === 1 && chosen[0] === id;

  return (
    <Modal
      icon="layers"
      title="Choose a Gate model"
      subtitle={`${appName} may use any model you enable here`}
      closeButton
      secondary={
        !loading && !failure ? { label: "Cancel", onClick: onDismiss } : undefined
      }
      primary={
        !loading && !failure
          ? {
              label: "Save models",
              onClick: () => onSave(draft),
              // Gate cannot serve a model nobody enabled, so an empty set is not
              // a saveable state. AG-590's "the final model cannot be removed"
              // is enforced on the row itself; this is the backstop for a set
              // that started empty.
              disabled: draft.length === 0,
            }
          : undefined
      }
      onDismiss={onDismiss}
      initialFocus={searchRef}
    >
      {loading ? (
        <div className="flex flex-col gap-1" aria-busy>
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-9" />
          ))}
        </div>
      ) : failure ? (
        <ModalNote>
          <p className="font-medium text-base-foreground">
            Gate could not list its models
          </p>
          <p className="mt-1">
            Nothing has changed: this app keeps the model it is using. Close
            this and try again.
          </p>
        </ModalNote>
      ) : models.length === 0 ? (
        <ModalNote>
          <p className="font-medium text-base-foreground">
            No models to choose from yet
          </p>
          <p className="mt-1">
            This gateway offers no models of its own, so apps keep using the
            model they are configured with.
          </p>
        </ModalNote>
      ) : (
        <>
          {/* Search and provider filter (Figma 139:66683). Both are client-side
           *  over the catalogue already in hand - the endpoint takes no query, and
           *  344 rows filter faster than a round trip. */}
          <div className="flex items-center gap-3">
            <label className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-sm border border-base-input bg-base-card px-2.5 shadow-base-xs focus-within:outline focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-base-primary">
              <Icon
                name="search"
                size={16}
                className="shrink-0 text-neutral-500"
              />
              <input
                ref={searchRef}
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search models"
                aria-label="Search models"
                className="w-full bg-transparent text-sm leading-5 text-base-foreground outline-none placeholder:text-neutral-500"
              />
            </label>
            <select
              value={vendor}
              onChange={(e) => setVendor(e.target.value)}
              aria-label="Provider"
              className="h-9 shrink-0 rounded-sm border border-base-input bg-base-card px-2.5 text-sm font-medium leading-5 text-base-foreground shadow-base-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
            >
              <option value="all">All providers</option>
              {vendors.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </select>
          </div>

          {/* The frame reads "Showing 10 of 14 models・400+ in Gate AI". The third
           *  clause distinguishes what this tool may use from everything Gate
           *  offers, and nothing filters per tool yet - so the two numbers would
           *  be the same and saying it twice would imply a filter that is not
           *  running. Reinstate it with AG-590's per-tool filtering. */}
          <p className="text-base-xs leading-4 text-base-muted-foreground">
            Showing {shown.length} of {models.length} models
          </p>

          {missing.length > 0 && (
            <ul className="flex flex-col gap-px">
              {missing.map((id) => {
                const locked = wouldEmpty(id);
                return (
                  <li key={id}>
                    <button
                      type="button"
                      role="checkbox"
                      aria-checked
                      aria-disabled={locked || undefined}
                      title={
                        locked
                          ? "Gate needs at least one model. Choose another first, or switch this app back to App default."
                          : undefined
                      }
                      onClick={() => {
                        if (locked) return;
                        setDraft((d) => d.filter((x) => x !== id));
                      }}
                      className={`flex w-full items-center gap-3 rounded-base border border-amber-300 bg-amber-50 px-3 py-2 text-left ${
                        locked ? "cursor-not-allowed" : ""
                      }`}
                    >
                      <Icon
                        name="triangleAlert"
                        size={16}
                        className="shrink-0 text-amber-700"
                      />
                      <span className="min-w-0 flex-1 truncate font-mono text-sm leading-5 text-amber-900">
                        {id}
                      </span>
                      <span className="shrink-0 text-base-2xs uppercase leading-4 tracking-label text-amber-800">
                        Unavailable
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}

          {shown.length === 0 ? (
            <ModalNote>
              <p>No model matches that search.</p>
            </ModalNote>
          ) : (
            <div
              role="group"
              aria-label="Gate model"
              className="-mr-1 flex max-h-[26rem] flex-col gap-px overflow-y-auto pr-1"
            >
              {shown.map((model) => {
                const selected = chosen.includes(model.id);
                const locked = selected && wouldEmpty(model.id);
                return (
                  <button
                    key={model.id}
                    type="button"
                    role="checkbox"
                    aria-checked={selected}
                    aria-disabled={locked || undefined}
                    title={
                      locked
                        ? "Gate needs at least one model. Choose another first, or switch this app back to App default."
                        : undefined
                    }
                    onClick={() => {
                      if (locked) return;
                      setDraft((d) =>
                        d.includes(model.id)
                          ? d.filter((x) => x !== model.id)
                          : [...d, model.id],
                      );
                    }}
                    className={`flex shrink-0 items-center gap-3 rounded-sm border px-3 py-2 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
                      selected
                        ? "border-base-primary bg-base-card"
                        : "border-transparent hover:bg-gray-50"
                    } ${locked ? "cursor-not-allowed" : ""}`}
                  >
                    <span
                      aria-hidden
                      className="flex size-4 shrink-0 items-center justify-center"
                    >
                      {model.logo ?? <Icon name="cube" size={16} />}
                    </span>
                    <span className="min-w-0 flex-1 truncate font-mono text-sm leading-5 text-base-foreground">
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
          )}

          {/* AG-590 asks that the set be stated before confirmation, and that
           *  the cost consequence be stated with it. */}
          <ModalNote>
            <p className="font-medium text-base-foreground">
                {draft.length === 1
                  ? "1 model enabled"
                  : `${draft.length} models enabled`}
              </p>
              <p className="mt-1">
                Eligible requests may use any of them and consume Gate credits.
                Gate never uses a model you have not enabled.
              </p>
          </ModalNote>
        </>
      )}
    </Modal>
  );
}

export function UseGateModelDialog({
  app,
  vendor,
  modelIds,
  /** Pre-formatted balance, e.g. "$10.25 available". */
  credits,
  vendorLogo,
  onKeepAppDefault,
  onUseGateCredits,
}: {
  app: DialogApp;
  /** Who makes the model, shown only when there is one to attribute (Figma
   *  130:48278 draws "Anthropic" above the id). */
  vendor: string;
  /**
   * Every model this switch enables, in the user's order (AG-590).
   *
   * One entry draws the frame exactly: vendor mark, vendor, id, PAYG pill.
   * Several stack the ids in the same row with the mark dropped - a column of
   * marks would imply each id belongs to the one beside it, which is only true
   * by accident, and the row is where the reader is already looking for what
   * they are about to pay for. It replaces an "Also enabled" note that put half
   * the set in one place and half in another.
   */
  modelIds: string[];
  credits: string;
  vendorLogo?: ReactNode;
  onKeepAppDefault: () => void;
  onUseGateCredits: () => void;
}) {
  const single = modelIds.length === 1;
  return (
    <Modal
      icon="layers"
      title={`Use a Gate model for ${app.name}?`}
      subtitle="Your next requests will use Constellation Gate PAYG credits"
      secondary={{ label: "Keep App default", onClick: onKeepAppDefault }}
      primary={{ label: "Use Gate credits", onClick: onUseGateCredits }}
      onDismiss={onKeepAppDefault}
      // 130:48278 draws this one narrower than the picker; 512 is one of the
      // four widths the file uses.
      width={512}
    >
      {single ? (
        <ModalSubject
          icon={vendorLogo ?? <Icon name="cube" size={16} />}
          title={vendor}
          description={modelIds[0]}
          variant="identity"
          pill={{ label: "PAYG", tone: "neutral" }}
        />
      ) : (
        // The same row, holding a set. No mark: one glyph cannot stand for
        // several vendors, and repeating it per line would say each id is that
        // vendor's when the set is usually mixed.
        <div className="flex items-start gap-3 rounded-md border border-base-border p-3">
          <ul className="flex min-w-0 flex-1 flex-col gap-1">
            {modelIds.map((id) => (
              <li
                key={id}
                className="truncate font-mono text-sm leading-5 text-base-foreground"
              >
                {id}
              </li>
            ))}
          </ul>
          <span className="shrink-0 rounded-sm border border-base-border px-2 py-1 font-mono text-base-2xs leading-4 text-neutral-700">
            PAYG
          </span>
        </div>
      )}

      {/* Credits and the reassurance are one block, as the frame draws them
       * (130:48302): the balance is the thing being spent and the sentence is
       * what limits the commitment, so they belong to each other rather than
       * reading as two unrelated notes. */}
      <div className="rounded-md bg-gray-50 p-3">
        <div className="flex items-center gap-3">
          <span
            aria-hidden
            className="flex size-9 shrink-0 items-center justify-center rounded-sm border border-base-border bg-base-card text-neutral-700"
          >
            <Icon name="creditCard" size={20} />
          </span>
          <p className="flex-1 text-sm leading-5 text-neutral-600">
            Gate credits:
          </p>
          <p className="shrink-0 text-sm font-medium leading-5 text-base-foreground">
            {credits}
          </p>
        </div>
        <p className="mt-3 text-sm leading-5 text-neutral-600">
          {app.name}&apos;s own model preference is not changed. You can return
          to App default at any time.
        </p>
      </div>
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
      // `Monitor` (`143:70310`), matching the Device row that opens this. The
      // 2026-08-26 sweep fixed the row and left the dialog on the near-miss.
      icon="monitor"
      tile="sm"
      title="Rename your device"
      width={480}
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Rename device",
        onClick: onRename,
        disabled: !newName.trim(),
      }}
      onDismiss={onCancel}
      initialFocus={field}
    >
      <ModalField label="Current device name" value={currentName} readOnly />
      <ModalField
        label="New device name"
        value={newName}
        onChange={onNewNameChange}
        maxLength={DEVICE_NAME_MAX_LENGTH}
        inputRef={field}
      />
    </Modal>
  );
}

/**
 * Replace the API key.
 *
 * `177:74869` labels the second field "New device name", copy-pasted from the
 * rename dialog. Shipped as "New API key" by explicit decision (2026-08-26),
 * standing as the named exception to "the file wins": the drawn label would put
 * a wrong word on the one screen where the user handles a credential. Raised
 * with the designer.
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
      tile="sm"
      title="Replace API key"
      width={480}
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Replace key",
        onClick: onReplace,
        disabled: !newKey.trim(),
      }}
      onDismiss={onCancel}
      initialFocus={field}
    >
      <ModalField
        label="Current API key"
        value={currentKeyMasked}
        readOnly
        mono
      />
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
      // 32px with a 16px glyph (`143:70620`), like its two 480px siblings -
      // the red tile the build drew at 44 was the loudest of the tile misses.
      tile="sm"
      title="Disconnect Gate?"
      width={480}
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: "Yes, disconnect Gate",
        onClick: onDisconnect,
        destructive: true,
      }}
      onDismiss={onCancel}
      edge="danger"
    >
      {/* `164:73502` reads "Protection turns off, your apps stop routing through
       * Gate, and your API key is removed from the keychain" - which describes
       * Reset, the row below this one on the same screen. Corrected by explicit
       * decision (2026-08-26), the second named exception to "the file wins":
       * disconnecting ends the session and touches no keychain item, so the
       * drawn sentence promises a change this action does not make. The ink is
       * still the frame's `base/foreground`; only the words are ours. */}
      <p className="text-sm leading-5 text-base-foreground">
        This device signs out of Gate and stops sending activity. Your apps keep
        their current configuration, and signing back in restores routing.
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
      width={544}
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
            description:
              "Managed tools return to their saved pre_gate configurations.",
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
 * **Nothing renders this today.** Its Settings row was removed on 2026-08-27
 * for being undrawn, which took AG-603's only surface with it. Kept rather than
 * deleted because the list itself is the expensive part - it is written from
 * what `analytics.ts` actually sends, not from the ticket's field list - and
 * because the criterion has not been withdrawn, only left without a door.
 *
 * Opened from a link inside the share-diagnostics row's own description rather
 * than from a row of its own: the file draws two rows under Diagnostics, and a
 * disclosure about a setting reads better as part of that setting's sentence
 * than as furniture beside it.
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
        <p className="font-medium text-base-foreground">Sent</p>
        <ul className="mt-1 list-disc pl-4">
          <li>
            An anonymous device id, generated locally. No name, email, or
            account identifier.
          </li>
          <li>App version and operating system.</li>
          <li>
            Which action happened, from a fixed list - routing turned on or off,
            an update installed, a dialog shown. Never free text.
          </li>
          <li>
            A short label for each action: which app or provider it concerned,
            and whether it was on or off.
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
        <p className="font-medium text-base-foreground">
          Sent with your traffic, whatever this setting says
        </p>
        <ul className="mt-1 list-disc pl-4">
          <li>
            The same anonymous device id, so your activity view can group
            requests by machine. It identifies nothing else and authorizes
            nothing.
          </li>
          <li>
            Which app made the request, when Gate can tell from the request
            itself - Claude Code, Codex, and so on. Unrecognised apps are sent
            unlabelled rather than guessed at.
          </li>
        </ul>
      </Wrapper>
      <Wrapper>
        <p className="font-medium text-base-foreground">Never sent</p>
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

/**
 * What the interrupted operation did, tool by tool.
 *
 * **Read-only, and deliberately so.** AG-570 requires that reviewing details "does
 * not change state" - so the only action closes it, and nothing here can be
 * clicked into a retry. Resuming and retrying are the notice's job, one surface
 * up, which is also where the buttons are.
 *
 * What it shows, per the same AC: the stages that completed and the ones still
 * pending, the *category* of each failure rather than an error string, the last
 * check that concluded for each tool, and enough per-tool diagnostics to hand to
 * someone else - the write's stage and age, the last verified route, the most
 * recent check, and whether a process is holding older settings.
 *
 * Nothing here is sensitive: slugs, display names, four closed vocabularies and
 * timestamps. No paths, no URLs, no credentials, no request content. That is what
 * makes it safe to show in full, and it is a property of the DTO rather than of
 * this component - see `recovery_summary`'s own docs.
 *
 * **Provisional layout.** The Figma draws no details view (AG-569 is To Do).
 */
export function RestoreDetailsDialog({
  summary,
  now,
  onClose,
}: {
  summary: RecoverySummary;
  /** One clock for the whole dialog, passed in so two rows cannot disagree
   *  about what "4m ago" means. */
  now: Date;
  onClose: () => void;
}) {
  const rows = recoveryRows(summary, now);
  const counts = stageCounts(summary);
  const failures = rows.filter((r) => r.errorCategory.length > 0);
  return (
    <Modal
      tone="neutral"
      icon="info"
      title="What happened to routing"
      subtitle={operationLine(summary, now)}
      primary={{ label: "Close", onClick: onClose }}
      onDismiss={onClose}
      width={600}
    >
      {rows.length === 0 ? (
        <ModalNote>Nothing was recorded for this attempt.</ModalNote>
      ) : (
        <>
          <ModalNote>
            <p className="font-medium text-base-foreground">
              {counts.complete} of {counts.total} stages completed
              {counts.pending > 0 ? `, ${counts.pending} still pending` : ""}
            </p>
            {failures.length > 0 && (
              // Categories, not messages. The restore branches on these
              // conditions itself, so the grouping cannot drift from the
              // attempt the way a parsed error string would.
              <p className="mt-1">
                Failures by category:{" "}
                {[...new Set(failures.map((r) => r.errorCategory))].join(", ")}.
              </p>
            )}
          </ModalNote>
          {rows.map((row) => (
            <RecoveryDetailRow key={`${row.kind}:${row.slug}`} row={row} />
          ))}
        </>
      )}
    </Modal>
  );
}

/** One tool's four readings, laid out as a definition list so the labels stay
 *  legible when a value wraps. Not `ModalSubject`: that row truncates its
 *  description to one line, which is right for naming a subject and wrong for a
 *  diagnostic block. */
function RecoveryDetailRow({ row }: { row: RecoveryRow }) {
  return (
    <div className="rounded-md border border-base-border p-3">
      <div className="flex items-baseline justify-between gap-2">
        <p className="truncate text-sm font-medium leading-5 text-base-foreground">
          {row.name}
        </p>
        <span
          className={`shrink-0 rounded-sm px-1.5 py-0.5 font-mono text-base-2xs leading-4 ${
            row.stageComplete
              ? "bg-green-100 text-green-900"
              : "bg-amber-100 text-amber-900"
          }`}
        >
          {row.stage}
        </span>
      </div>
      <p className="mt-1 text-sm leading-5 text-neutral-600">{row.stageDetail}</p>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-base-xs leading-4">
        <dt className="text-base-muted-foreground">Stage</dt>
        <dd className="text-base-foreground">{row.stageLine}</dd>
        {row.errorCategory && (
          <>
            <dt className="text-base-muted-foreground">Failure</dt>
            <dd className="text-base-foreground">{row.errorCategory}</dd>
          </>
        )}
        <dt className="text-base-muted-foreground">Last verified route</dt>
        <dd className="text-base-foreground">{row.lastVerified ?? "No reading yet"}</dd>
        <dt className="text-base-muted-foreground">Last check</dt>
        <dd className="text-base-foreground">{row.checkResult}</dd>
        <dt className="text-base-muted-foreground">Process</dt>
        <dd className="text-base-foreground">{row.runningState}</dd>
        <dt className="text-base-muted-foreground">Next action</dt>
        <dd className="text-base-foreground">{row.action ?? "Nothing to do"}</dd>
      </dl>
    </div>
  );
}

/**
 * Where every tool stands after a teardown - routing off, disconnect, sign-out
 * or reset.
 *
 * AG-570 asks for this whenever such an operation "cannot write defaults for
 * every tool": the tools that are back on their own settings, the ones still
 * carrying Gate's, the ones waiting to be reopened, the ones that could not be
 * read, and what to do about each. The four buckets are the answer, and they are
 * *read back* from the configs rather than assembled from what the teardown
 * believed it wrote - a sweep that reports success having written nothing is the
 * failure this dialog exists to catch.
 *
 * Read-only for the same reason the review above is: it reports a teardown that
 * has already happened. The actions it names live on the rows that own them.
 */
export function TeardownReportDialog({
  report,
  onClose,
}: {
  report: TeardownReport;
  onClose: () => void;
}) {
  const outstanding =
    report.still_gate.length + report.awaiting_reopen.length + report.failed.length;
  const sections: {
    key: keyof TeardownReport;
    title: string;
    detail: string;
    tone: PillTone;
  }[] = [
    {
      key: "still_gate",
      title: "Still using Gate’s values",
      detail: "The teardown could not put these back. Their config still points at Gate.",
      tone: "amber",
    },
    {
      key: "awaiting_reopen",
      title: "Waiting to be reopened",
      detail:
        "Back on their own settings on disk, but a running process is still using the route it started with.",
      tone: "amber",
    },
    {
      key: "failed",
      title: "Could not be checked",
      detail:
        "Gate could not read these configs, so nothing about them is known - which is not the same as clean.",
      tone: "amber",
    },
    {
      key: "defaults",
      title: "Back on their own settings",
      detail: "Verified by reading the config, not by trusting the write.",
      tone: "green",
    },
  ];
  return (
    <Modal
      tone={outstanding > 0 ? "warning" : "success"}
      icon={outstanding > 0 ? "triangleAlert" : "circleCheck"}
      title={outstanding > 0 ? "Some tools were left as they were" : "Every tool is back on its own settings"}
      subtitle={
        outstanding > 0
          ? `${outstanding} of ${outstanding + report.defaults.length} tools still need something.`
          : "Nothing is left pointing at Gate."
      }
      primary={{ label: "Close", onClick: onClose }}
      onDismiss={onClose}
      width={544}
    >
      {sections
        .filter((section) => report[section.key].length > 0)
        .map((section) => (
          <div key={section.key} className="flex flex-col gap-2">
            <p className="text-base-xs font-medium leading-4 text-base-muted-foreground">
              {section.title}
            </p>
            {report[section.key].map((tool) => (
              <ModalSubject
                key={tool.slug}
                icon="cube"
                title={tool.name}
                description={section.detail}
                pill={
                  tool.next_action === "none"
                    ? { label: "Done", tone: section.tone }
                    : {
                        label: TEARDOWN_ACTION_LABEL[tool.next_action],
                        tone: section.tone,
                      }
                }
              />
            ))}
          </div>
        ))}
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
 * Quit, step one: choose *how*, from the two outcomes AG-596 names.
 *
 * Drawn at last (`Flows / Overview`, `overview-quit` 694:31955, read
 * 2026-08-28), and the drawing replaces the three-button dialog this shipped as.
 * The two outcomes are selectable rows now, with the safe one recommended by a
 * pill and preselected, and the primary carries out the choice; the third
 * outcome, not quitting, stays a Cancel button because it is not a way to quit.
 *
 * The step after this is `QuitSafeToCloseDialog`, which is what actually
 * closes the app. So neither button here exits: `Disconnect` puts the tools
 * back and reports, `Continue` goes straight to the same report. That is what
 * lets the confirmation speak in the past tense, as drawn.
 *
 * Focus opens on the recommended choice rather than on a button: the user asked
 * to quit, but Enter on a panel they have not read should not decide *how*, and
 * from the radio it can only re-select what is already selected.
 *
 * The popover's `QuitConfirm` keeps the older three-button shape. The two
 * shells disagree until the popover retires, the same way they already disagree
 * about the org-picker dead end - this is the drawn one.
 */
export type QuitChoice = "disconnect" | "leave";

export function QuitDialog({
  tools,
  choice,
  onChoose,
  busy,
  onContinue,
  onCancel,
}: {
  /** Config-routed tools still pointed at Gate. Never empty - the shell only
   * raises this dialog when the backend reports at least one. */
  tools: string[];
  choice: QuitChoice;
  onChoose: (next: QuitChoice) => void;
  busy?: boolean;
  onContinue: () => void;
  onCancel: () => void;
}) {
  const plural = tools.length > 1;
  return (
    <Modal
      tone="warning"
      icon="triangleAlert"
      title="Quit Gate Connect?"
      subtitle={
        <>
          {/* The count is drawn in Medium inside a regular sentence: it is the
              figure the sentence is about. */}
          <span className="font-medium">{tools.length}</span> protected app
          {plural ? "s are" : " is"} still routed through Gate
        </>
      }
      secondary={{ label: "Cancel", onClick: onCancel, disabled: busy }}
      primary={{
        // Named for what it does, which is why it changes with the choice: the
        // drawn "Disconnect" belongs to the drawn selection, and leaving it
        // there under "Quit without disconnecting" would label a button with
        // the opposite of its action. The second label is inferred - the frame
        // draws only the first row selected.
        label: busy
          ? "Working…"
          : choice === "disconnect"
            ? "Disconnect"
            : "Continue",
        onClick: onContinue,
        disabled: busy,
      }}
      onDismiss={busy ? undefined : onCancel}
    >
      <p className="text-sm font-medium leading-5 text-base-muted-foreground">
        Select how you want to quit the app
      </p>
      <div role="radiogroup" aria-label="How to quit" className="flex flex-col gap-2">
        <ModalChoice
          title="Disconnect tools and quit"
          description="Restore saved configurations, turn routing off, then quit."
          pill="Safest"
          selected={choice === "disconnect"}
          onSelect={() => onChoose("disconnect")}
        />
        <ModalChoice
          title="Quit without disconnecting"
          description="Leave configurations pointed at Gate. Requests that depend on the local proxy may pause."
          selected={choice === "leave"}
          onSelect={() => onChoose("leave")}
        />
      </div>
      <ModalNote tone="info">
        <span className="font-medium">
          Closing the main window is a different action.
        </span>{" "}
        You can safely <span className="font-medium">minimize</span> this app to
        the menu bar to keep protection running quietly.
      </ModalNote>
    </Modal>
  );
}

/**
 * Quit, step two: what happened, and the button that actually closes the app
 * (`694:33002` after disconnecting, `694:33340` after leaving things in place).
 *
 * The two notes are the drawn copy for the two branches, and they are reports
 * rather than promises - which is why this dialog is only reached when the
 * branch it describes has already run. A teardown that left something behind
 * gets `QuitLeftBehindDialog` instead: "their previous settings are restored"
 * would be exactly the claim AG-596 forbids.
 *
 * Cancel stays as drawn even though the work is done by now. Someone who
 * changes their mind here keeps a running app whose tools have been
 * disconnected, which the note has just told them in as many words.
 */
export function QuitSafeToCloseDialog({
  disconnected,
  busy,
  onClose,
  onCancel,
}: {
  /** Which branch got here: the teardown ran cleanly, or nothing was touched. */
  disconnected: boolean;
  busy?: boolean;
  onClose: () => void;
  onCancel: () => void;
}) {
  return (
    <Modal
      tone="success"
      icon="circleCheck"
      tile="sm20"
      width={536}
      title="Safe to close Gate Connect"
      secondary={{ label: "Cancel", onClick: onCancel, disabled: busy }}
      primary={{
        label: busy ? "Working…" : "Close Gate Connect",
        onClick: onClose,
        disabled: busy,
      }}
      onDismiss={busy ? undefined : onCancel}
    >
      <ModalNote tone="neutral">
        {disconnected
          ? "Tools are disconnected and their previous settings are restored. Setup will be waiting the next time you open the app."
          : "Routing settings were left in place. Some tools may need Gate Connect running to complete requests."}
      </ModalNote>
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
      primary={{
        label: busy ? "Working…" : "Try again",
        onClick: onRetry,
        disabled: busy,
      }}
      onDismiss={busy ? undefined : onCancel}
    >
      <p className="text-sm leading-5 text-neutral-600">
        Couldn’t put {joinNames(tools)} back on{" "}
        {plural ? "their own settings" : "its own settings"}.{" "}
        {plural ? "They still point" : "It still points"} at Gate, and won’t
        reach a model until Gate Connect runs again.
      </p>
      <ModalNote>
        Everything else was put back. Trying again only retouches the{" "}
        {plural ? "tools" : "tool"} above.
      </ModalNote>
    </Modal>
  );
}
