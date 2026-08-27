import { Fragment } from "react";
import { BaseSwitch, Card } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";
import type { AuthMode } from "../../lib/api";

/**
 * The Settings pane (Figma `Flows / Settings`). Named `SettingsPane` rather
 * than `Settings` so it does not read as a swap for `screens/Settings.tsx`,
 * which is the popover's version and stays until the shell swap.
 *
 * Rows are uniform enough across all six sections that the pane takes a
 * declarative model rather than a prop per field: label, an optional value, and
 * either an action button or a switch. Presentational, like the rest.
 */

export interface SettingsAction {
  label: string;
  onClick: () => void;
  /** Filled red rather than outline: Disconnect Gate, Review reset. */
  destructive?: boolean;
  /** Opens the web dashboard, so the label carries the external-link glyph. */
  external?: boolean;
}

export interface SettingsRow {
  id: string;
  icon: IconName;
  label: string;
  /** Second line under the label - the Startup rows and Danger zone. */
  description?: string;
  /** Middle column, e.g. "MacBook Pro". */
  value?: string;
  action?: SettingsAction;
  toggle?: { on: boolean; onToggle: () => void };
  /**
   * This row's value could not be read. Renders "Unavailable" and a Retry in
   * place of the value and control, rather than showing a default dressed as
   * fact.
   *
   * The distinction matters most on a switch: a failed read that falls back to
   * `false` draws an Off switch, which is a claim about the user's setting, and
   * the user cannot tell it from a setting they turned off themselves. Same
   * argument as `Overview`'s zeroed metrics and `lib/verdict.ts`'s refusal to
   * infer routing from a config file.
   */
  unavailable?: { onRetry: () => void };
}

export interface SettingsSection {
  id: string;
  title: string;
  /** Red heading and a red-tinted card. */
  danger?: boolean;
  rows: SettingsRow[];
}

/**
 * The design's own section list, as data. Kept here rather than in the shell so
 * the copy stays next to the screen it describes, and so "which rows exist" is
 * testable without mounting the app.
 *
 * Diagnostics is the one row the Figma does not draw. `screens/Diagnostics.tsx`
 * exists and has nowhere else to live in the new IA, so it sits under About and
 * opens the report dialog. Expect it to be redrawn.
 *
 * **An omitted handler omits its control**, and a row left with nothing to do
 * omits itself. What is still missing a backend is device rename and plan
 * upgrade, which has no billing URL to open, plus Support, which has no address.
 * A switch or button that visibly does nothing is worse than an absent one: the
 * user cannot tell "not built" from "broken", and on the Danger zone card they
 * cannot tell it from "already done".
 */
export function buildSettingsSections({
  deviceName,
  installId,
  loginId,
  plan,
  gateway,
  apiKeyMasked,
  authMode,
  launchAtLogin,
  launchAtLoginUnavailable,
  routingHealthNotifications,
  shareDiagnostics,
  preferencesUnavailable,
  version,
  updateNote,
  certificate,
  onChangeGateway,
  onRemoveCertificate,
  onRenameDevice,
  onCopyInstallId,
  onUpgradePlan,
  onReplaceKey,
  onSwitchToGateAccount,
  signInNote,
  onDisconnect,
  onToggleLaunchAtLogin,
  onRetryLaunchAtLogin,
  onToggleRoutingHealthNotifications,
  onToggleShareDiagnostics,
  onRetryPreferences,
  onReplayTutorial,
  onCheckForUpdates,
  onViewDiagnostics,
  onViewCollectedData,
  onOpenDocs,
  onContactSupport,
  onReviewReset,
}: {
  deviceName: string;
  installId: string;
  loginId: string;
  plan: string;
  gateway: string;
  /** Already masked upstream - this pane never sees the key. */
  apiKeyMasked: string;
  /**
   * How the account actually authenticates, which decides whether the API key
   * row belongs on screen at all.
   *
   * An OAuth account keeps whatever key it had before the upgrade: the keychain
   * item is not deleted, so `has_api_key` stays true and this pane used to draw
   * a masked key under "API key" for a session authenticated by a Cognito
   * bearer. On the screen whose whole job is to say where the credential lives,
   * that named the wrong one - and the key it named cannot be replaced from
   * here either, because `onReplaceKey` is withheld for OAuth.
   *
   * Undefined leaves the key row in place: it is the state before the account
   * read lands, and the key row is the older default.
   */
  authMode?: AuthMode;
  launchAtLogin: boolean;
  /** The launch-at-login read failed. Drives the Unavailable row; the boolean
   * above is then meaningless and must not reach a switch. */
  launchAtLoginUnavailable?: boolean;
  routingHealthNotifications?: boolean;
  shareDiagnostics?: boolean;
  /** The preferences read failed - same reasoning as `launchAtLoginUnavailable`,
   * for the two switches that come from `preferences.json`. */
  preferencesUnavailable?: boolean;
  version: string;
  /** Feedback under the version row after an explicit update check. */
  updateNote?: string;
  /** Whether the Gate certificate is in the system trust store, as a phrase.
   * Absent on a platform with no proxy subsystem, which has no certificate. */
  certificate?: string;
  /** Dev builds only: repoint the account at another environment. */
  onChangeGateway?: () => void;
  /** Only offered while the certificate is actually trusted; removing one that
   * is not there is a button that cannot do anything. */
  onRemoveCertificate?: () => void;
  onRenameDevice?: () => void;
  onCopyInstallId: () => void;
  onUpgradePlan?: () => void;
  onReplaceKey?: () => void;
  /** Offered only to an account still on a pasted key. The popover has carried
   * this since it shipped (`screens/Settings.tsx`); the new shell had only the
   * one-time `OAuthOfferDialog`, and `markOAuthOfferSeen` meant that dismissing
   * it once left no route to a Gate account at all. */
  onSwitchToGateAccount?: () => void;
  /** Replaces that row's description while the browser flow is open, the same
   * way `updateNote` speaks for the version row. */
  signInNote?: string;
  onDisconnect?: () => void;
  onToggleLaunchAtLogin: () => void;
  /** Present only when the launch-at-login read failed, so the row can offer a
   * retry instead of drawing a switch from a value it does not have. */
  onRetryLaunchAtLogin?: () => void;
  onToggleRoutingHealthNotifications?: () => void;
  onToggleShareDiagnostics?: () => void;
  onRetryPreferences?: () => void;
  onReplayTutorial: () => void;
  onCheckForUpdates?: () => void;
  onViewDiagnostics: () => void;
  /** Opens the collected-data list. Read-only: AG-603 requires it to open
   * "without changing the setting". */
  onViewCollectedData?: () => void;
  onOpenDocs?: () => void;
  onContactSupport?: () => void;
  onReviewReset?: () => void;
}): SettingsSection[] {
  return [
    {
      id: "device",
      title: "Device",
      rows: [
        {
          id: "device",
          icon: "monitor",
          label: "Device",
          value: deviceName,
          action: onRenameDevice
            ? { label: "Rename device", onClick: onRenameDevice }
            : undefined,
        },
        {
          id: "install-id",
          icon: "squareUser",
          label: "Install ID",
          value: installId,
          action: { label: "Copy ID", onClick: onCopyInstallId },
        },
      ],
    },
    {
      id: "account",
      title: "Account",
      rows: [
        { id: "login", icon: "userRound", label: "Login ID", value: loginId },
        {
          id: "plan",
          icon: "fileBadge2",
          label: "Gate plan",
          value: plan,
          action: onUpgradePlan
            ? { label: "Upgrade plan", onClick: onUpgradePlan, external: true }
            : undefined,
        },
      ],
    },
    {
      id: "connection",
      title: "Connection",
      rows: [
        {
          id: "gateway",
          icon: "globe",
          label: "Gateway",
          value: gateway,
          action: onChangeGateway
            ? { label: "Change server", onClick: onChangeGateway }
            : undefined,
        },
        // Only for an account a key actually authenticates. See `authMode`.
        ...(authMode === "oauth"
          ? []
          : [
              {
                id: "api-key",
                icon: "key" as IconName,
                label: "API key",
                value: apiKeyMasked,
                action: onReplaceKey
                  ? { label: "Replace key", onClick: onReplaceKey }
                  : undefined,
              } as SettingsRow,
            ]),
        // Takes the key row's place for a Gate account, so the section still
        // says what signs the user in rather than going quiet about it. The
        // switch action stays gated on the handler, which the shell withholds
        // for an account already on OAuth - there is nowhere to switch to.
        ...(authMode === "oauth"
          ? [
              {
                id: "sign-in-method",
                icon: "shieldCheck" as IconName,
                label: "Sign-in method",
                description:
                  signInNote ??
                  "Your Gate account keeps its session in the OS secret store and refreshes on its own, so there is no key to paste or rotate.",
                value: "Gate account",
              } as SettingsRow,
            ]
          : onSwitchToGateAccount
            ? [
                {
                  id: "sign-in-method",
                  icon: "shieldCheck" as IconName,
                  label: "Sign-in method",
                  description:
                    signInNote ??
                    "A Gate account keeps its session in the OS secret store and refreshes on its own, so there is nothing to paste or rotate.",
                  value: "API key",
                  action: {
                    label: "Use a Gate account",
                    onClick: onSwitchToGateAccount,
                  },
                } as SettingsRow,
              ]
            : []),
        ...(certificate
          ? [
              {
                id: "certificate",
                icon: "shieldCheck" as IconName,
                label: "Gate certificate",
                description:
                  "Lets Gate inspect your AI traffic locally. Removing it stops inspection until it is trusted again.",
                value: certificate,
                // Red, like Disconnect and Reset: it is reversible, but until it is
                // reversed every routed domain stops being inspected, and that is
                // the consequence the user is deciding about. Three red actions on
                // one screen, where CLAUDE.md says to question a third - questioned
                // and kept, because the alternative is an outline button that
                // silently stops the product doing its job.
                action: onRemoveCertificate
                  ? {
                      label: "Remove certificate",
                      onClick: onRemoveCertificate,
                      destructive: true,
                    }
                  : undefined,
              } as SettingsRow,
            ]
          : []),
        ...(onDisconnect
          ? [
              {
                id: "session",
                icon: "link" as IconName,
                label: "Active session",
                action: {
                  label: "Disconnect Gate",
                  onClick: onDisconnect,
                  destructive: true,
                },
              },
            ]
          : []),
      ],
    },
    {
      id: "startup",
      title: "Startup",
      rows: [
        {
          id: "launch",
          icon: "circlePower",
          label: "Launch at login",
          description: "Keeps routing on after restart",
          ...(launchAtLoginUnavailable && onRetryLaunchAtLogin
            ? { unavailable: { onRetry: onRetryLaunchAtLogin } }
            : { toggle: { on: launchAtLogin, onToggle: onToggleLaunchAtLogin } }),
        },
        // A row under Startup, where the drawn screen keeps it ("Settings /
        // Main screens", read 2026-08-21) - an earlier build gave it a section
        // of its own because AG-594 names one.
        //
        // One switch, not the four the criteria list, and not the drawn
        // description either: the drawing promises alerts when a request is
        // blocked or flagged, and those events need the live security feed
        // (AG-578), which does not exist. The only notifications this app fires
        // are about routing itself, so that is what the row claims to control.
        ...(onToggleRoutingHealthNotifications
          ? [
              {
                id: "routing-health",
                icon: "bell" as IconName,
                label: "Notifications",
                // `116:29086`, verbatim. The honest routing-health wording was
                // ours, on the grounds that the blocked/flagged events need a
                // live security feed (AG-578) that does not exist. The file
                // wins; the row now promises more than it fires.
                description: "Alert me when a request is blocked or flagged",
                ...(preferencesUnavailable && onRetryPreferences
                  ? { unavailable: { onRetry: onRetryPreferences } }
                  : {
                      toggle: {
                        on: routingHealthNotifications ?? true,
                        onToggle: onToggleRoutingHealthNotifications,
                      },
                    }),
              } as SettingsRow,
            ]
          : []),
      ],
    },
    // Diagnostics gets its own section, out of About: sharing data is a privacy
    // choice, and the report is the evidence of what would be shared. Sending a
    // report on demand, and the reference it returns, belong with the collection
    // work and are not here.
    {
      id: "diagnostics",
      title: "Diagnostics",
      rows: [
        ...(onToggleShareDiagnostics
          ? [
              {
                id: "share-diagnostics",
                icon: "share2" as IconName,
                label: "Share diagnostic data",
                description:
                  "Send Gate errors and routing stats to help fix problems. Never prompts or credentials.",
                ...(preferencesUnavailable && onRetryPreferences
                  ? { unavailable: { onRetry: onRetryPreferences } }
                  : {
                      toggle: {
                        on: shareDiagnostics ?? true,
                        onToggle: onToggleShareDiagnostics,
                      },
                    }),
              } as SettingsRow,
            ]
          : []),
        ...(onViewCollectedData
          ? [
              {
                id: "collected-data",
                icon: "eye" as IconName,
                label: "What is collected",
                description: "The exact fields that leave this device, and the ones that never do",
                action: { label: "View list", onClick: onViewCollectedData },
              } as SettingsRow,
            ]
          : []),
        {
          id: "diagnostics-report",
          icon: "clipboardList",
          label: "Diagnostics report",
          description: "Everything Gate knows about this install, as shareable text.",
          action: { label: "View report", onClick: onViewDiagnostics },
        },
      ],
    },
    {
      id: "about",
      title: "About",
      rows: [
        {
          id: "tutorial",
          icon: "bookOpenText",
          label: "Tutorial",
          action: { label: "Replay tutorial", onClick: onReplayTutorial },
        },
        {
          id: "version",
          icon: "squareCode",
          label: "Version",
          description: updateNote,
          value: version,
          action: onCheckForUpdates
            ? { label: "Check for updates", onClick: onCheckForUpdates }
            : undefined,
        },
      ],
    },
    // Help is its own section because the criteria ask for it by name, and
    // because a documentation link buried under About reads as release notes.
    ...(onOpenDocs || onContactSupport
      ? [
          {
            id: "help",
            title: "Help",
            rows: [
              ...(onOpenDocs
                ? [
                    {
                      id: "docs",
                      icon: "bookOpenText" as IconName,
                      label: "Documentation",
                      description: "Setup, routing, and troubleshooting",
                      action: { label: "Read docs", onClick: onOpenDocs, external: true },
                    } as SettingsRow,
                  ]
                : []),
              ...(onContactSupport
                ? [
                    {
                      id: "support",
                      icon: "headset" as IconName,
                      label: "Support",
                      action: {
                        label: "Contact support",
                        onClick: onContactSupport,
                        external: true,
                      },
                    } as SettingsRow,
                  ]
                : []),
            ],
          },
        ]
      : []),
    // A Danger zone whose one action is inert is worse than no Danger zone: the
    // card is drawn to be alarming, and an alarming card that does nothing
    // teaches the user to ignore it.
    ...(onReviewReset
      ? [
          {
            id: "danger",
            title: "Danger zone",
            danger: true,
            rows: [
              {
                id: "reset",
                icon: "refresh" as IconName,
                label: "Reset Gate Connect",
                description:
                  "Turn routing off, disconnect tools, remove this account or key, and start setup again.",
                action: {
                  label: "Review reset",
                  onClick: onReviewReset,
                  destructive: true,
                },
              },
            ],
          },
        ]
      : []),
  ];
}

export function SettingsPane({ sections }: { sections: SettingsSection[] }) {
  return (
    <div className="flex flex-1 flex-col gap-6 overflow-auto bg-base-background p-6">
      {/* `heading/20` is 20/24 in the file. The token export's 28 is what
        * `tailwind.config.ts` records, and `text-xl` carries it by default, so
        * the leading is pinned here rather than left to the default. */}
      <h1 className="text-xl font-medium leading-6 tracking-heading text-base-foreground">
        Settings
      </h1>

      {sections.map((section) => (
        <section key={section.id} className="flex flex-col gap-3">
          <h2
            className={`text-base font-medium leading-6 tracking-heading ${
              section.danger ? "text-red-600" : "text-base-foreground"
            }`}
          >
            {section.title}
          </h2>

          {/* The card pads 16px and the rules sit inside that padding rather
           * than bleeding to its edges, which is what the frame draws: rows
           * are stacked at a 16px gap with a 1px rule between them. */}
          <Card
            className={`p-4 ${section.danger ? "border-red-600/40 bg-red-50" : ""}`}
          >
            <div className="flex flex-col gap-4">
              {section.rows.map((row, i) => (
                <Fragment key={row.id}>
                  {i > 0 && (
                    <div
                      className={`h-px ${
                        section.danger ? "bg-red-600/40" : "bg-base-border"
                      }`}
                    />
                  )}
                  <Row row={row} />
                </Fragment>
              ))}
            </div>
          </Card>
        </section>
      ))}
    </div>
  );
}

function Row({ row }: { row: SettingsRow }) {
  return (
    <div className="flex items-center gap-3">
      {/* Full-strength ink. Sampled off `191:79795` at #030712, where this had
        * been drawing `neutral-500` and reading washed out beside its label. */}
      <Icon name={row.icon} size={20} className="shrink-0 text-base-foreground" />

      <div
        className={`min-w-0 ${
          row.value === undefined && !row.unavailable ? "flex-1" : "w-[184px] shrink-0"
        }`}
      >
        <p className="truncate text-sm font-medium leading-5 text-base-foreground">
          {row.label}
        </p>
        {row.description && (
          <p className="text-base-xs leading-4 text-base-muted-foreground">
            {row.description}
          </p>
        )}
      </div>

      {row.unavailable ? (
        <>
          <p className="min-w-0 flex-1 truncate text-sm leading-5 text-base-muted-foreground">
            Unavailable
          </p>
          <ActionButton action={{ label: "Retry", onClick: row.unavailable.onRetry }} />
        </>
      ) : (
        <>
          {row.value !== undefined && (
            <p className="min-w-0 flex-1 truncate text-sm leading-5 text-base-foreground">
              {row.value}
            </p>
          )}

          {row.toggle && (
            <span className="flex shrink-0 items-center gap-2">
              <span className="text-sm leading-5 text-base-foreground">
                {row.toggle.on ? "On" : "Off"}
              </span>
              <BaseSwitch
                on={row.toggle.on}
                label={row.label}
                onClick={row.toggle.onToggle}
              />
            </span>
          )}

          {row.action && <ActionButton action={row.action} />}
        </>
      )}
    </div>
  );
}

function ActionButton({ action }: { action: SettingsAction }) {
  return (
    <button
      type="button"
      onClick={action.onClick}
      className={`flex h-8 shrink-0 items-center gap-1.5 rounded-control px-3 text-base-xs font-medium leading-4 tracking-button-xs transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
        action.destructive
          ? "bg-base-destructive text-base-destructive-foreground shadow-base-btn-destructive hover:bg-red-700 focus-visible:outline-red-600"
          : "border border-base-border bg-base-card text-base-primary shadow-base-btn-sm hover:bg-gray-50 focus-visible:outline-base-primary"
      }`}
    >
      {action.label}
      {action.external && <Icon name="squareArrowOutUpRight" size={16} />}
    </button>
  );
}
