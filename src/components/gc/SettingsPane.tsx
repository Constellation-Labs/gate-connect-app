import { Fragment } from "react";
import { BaseSwitch, Card, Skeleton } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";
import type { AuthMode } from "../../lib/api";
import { SHELL_CHANNEL_COVERAGE } from "../../lib/groups";

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
  /**
   * A link at the end of that second line.
   *
   * For a disclosure that belongs *to* a setting rather than beside it: AG-603's
   * field list had a row of its own, which the file does not draw, and a row is
   * too much furniture for "and here is exactly what that means". Inline, it
   * reads as part of the sentence it qualifies. Needs `description`.
   */
  descriptionLink?: { label: string; onClick: () => void };
  /** Middle column, e.g. "MacBook Pro". */
  value?: string;
  action?: SettingsAction;
  toggle?: {
    on: boolean;
    /**
     * An operation is in flight, so the switch keeps focus and ignores clicks.
     *
     * Optional because most rows here do not need it: a notification
     * preference or launch-at-login is a local write that lands before the
     * user can click twice. The shell-proxy row is the one that does - it
     * shares `useRouting`'s single `busy` flag with every app switch, and
     * `setEnvExport` opens with `if (busy) return`, so without this a click
     * made during any other routing operation is swallowed and the switch
     * simply does not move.
     */
    busy?: boolean;
    onToggle: () => void;
  };
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
  /**
   * The value is still being read. Draws a `Skeleton` in the value slot and
   * keeps the row in its value shape while it waits.
   *
   * Both halves matter. Principle 6 asks for a skeleton rather than a blank
   * where a reading is in flight, and without the flag `Row` cannot tell "not
   * yet" from a description row that has no value at all - so the label column
   * would take the full width, then snap back to 189px when the value landed,
   * carrying the trailing action across the row with it.
   */
  valuePending?: boolean;
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
 * Diagnostics was the one row the Figma did not draw, and it has since been
 * drawn: the file now carries a full Diagnostics section matching these rows
 * word for word. `screens/Diagnostics.tsx` sits under About and opens the
 * report dialog, which is where the frame puts it.
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
  planUnreadable,
  onRetryPlan,
  gateway,
  apiKeyMasked,
  authMode,
  launchAtLogin,
  launchAtLoginUnavailable,
  notifications,
  securityNotificationSound,
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
  shellProxy,
  onReplaceKey,
  onSwitchToGateAccount,
  signInNote,
  onDisconnect,
  onToggleLaunchAtLogin,
  onRetryLaunchAtLogin,
  onToggleNotifications,
  onToggleSecurityNotificationSound,
  onToggleShareDiagnostics,
  onViewCollectedData,
  onSendDiagnostics,
  onRetryPreferences,
  onReplayTutorial,
  onCheckForUpdates,
  onViewDiagnostics,
  onOpenDocs,
  onContactSupport,
  onReviewReset,
}: {
  deviceName: string;
  installId: string;
  loginId: string;
  /**
   * The org's plan, already in the user's vocabulary (`formatPlan`).
   *
   * `undefined` while the credits read is in flight, `null` when it landed and
   * named no plan. Both draw something other than a plan, and they are not the
   * same thing: the first is "not yet", the second is "the gateway did not
   * say". See `onRetryPlan`.
   */
  plan?: string | null;
  /** The credits read failed. Draws "Unavailable" and a Retry, as the other
   *  failed reads on this pane do. */
  planUnreadable?: boolean;
  onRetryPlan?: () => void;
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
  notifications?: boolean;
  securityNotificationSound?: boolean;
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
  /** The machine-wide shell proxy channel, or undefined where the platform
   *  cannot offer it separately (Linux, where these variables ARE the system
   *  proxy). The window owns this control; the tray reports it. */
  shellProxy?: { on: boolean; busy?: boolean; onToggle: () => void };
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
  onToggleNotifications?: () => void;
  onToggleSecurityNotificationSound?: () => void;
  onToggleShareDiagnostics?: () => void;
  /** Opens the collected-data list from the share-diagnostics row's own
   * description. Read-only: AG-603 requires it to open "without changing the
   * setting", which is also why it is a link and not a second switch. */
  onViewCollectedData?: () => void;
  /** Opens the send-one-report dialog. Optional like the switch above it, so a
   * build with no diagnostics destination configured omits the row rather than
   * offering a button that can only fail. */
  onSendDiagnostics?: () => void;
  onRetryPreferences?: () => void;
  onReplayTutorial: () => void;
  onCheckForUpdates?: () => void;
  onViewDiagnostics: () => void;
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
          // No second line, for the reason Sign-in method has none: `116:28986`
          // draws this row as icon, label, value and button on one 20px line,
          // and every drawn row that carries a value draws it the same way.
          //
          // What went with it was where the name goes - a chosen name rides
          // every proxied request as `x-gate-device-name`, a hostname fallback
          // goes nowhere - which is a principle-1 fact about the wire. The
          // Rename dialog is where it belongs if it comes back, since that is
          // where the user acts on it.
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
          // Four states, and the row must not flatten them (AG-891).
          //
          // This was the literal string "Unavailable", on a comment saying no
          // gateway field carried a plan. One does - `/v1/me/credits` reports
          // it, and the App pane had been drawing it since AG-592 - so Settings
          // said "Unavailable" while another pane in the same session named the
          // plan. Two screens, two sources, one account.
          //
          // `undefined` while the read is in flight leaves the value off rather
          // than guessing; `null` is a landed read that named no plan, which is
          // principle 6's "no figure without a reading" and draws the dash the
          // Login ID row uses for the same reason.
          value: plan ?? (plan === null ? "-" : undefined),
          valuePending: plan === undefined && !planUnreadable,
          ...(planUnreadable && onRetryPlan
            ? { unavailable: { onRetry: onRetryPlan } }
            : {}),
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
                // No standing description: every drawn row that carries a value
                // carries no second line, and the explanation this used to hold
                // wrapped the row to six of them. `signInNote` still lands here,
                // the way `updateNote` lands on Version - a transient line about
                // what is happening now, not a paragraph the row wears at rest.
                description: signInNote,
                value: "Gate account",
              } as SettingsRow,
            ]
          : onSwitchToGateAccount
            ? [
                {
                  id: "sign-in-method",
                  icon: "shieldCheck" as IconName,
                  label: "Sign-in method",
                  description: signInNote,
                  value: "API key",
                  action: {
                    label: "Use a Gate account",
                    onClick: onSwitchToGateAccount,
                  },
                } as SettingsRow,
              ]
            : []),
        // Machine-wide, so Settings rather than the app list - the rail is a
        // list of apps and this is not one (AG-893, and `isSettingsManaged`).
        // Placed in Connection because it decides HOW traffic reaches Gate,
        // which is what the rest of this section is about.
        //
        // Undrawn by the file: no frame carries this row, because the file draws
        // it as a card in the rail, which is where it should not be. The tray's
        // own card (`735:37341`) IS drawn and keeps its copy, as a status
        // display - the tray reports what the window decides.
        //
        // Absent on Linux, where these variables are the system proxy and cannot
        // be declined without turning routing off.
        ...(shellProxy
          ? [
              {
                id: "shell-proxy",
                icon: "squareCode" as IconName,
                label: "Command-line tools",
                // The two facts a reader needs and neither control used to
                // give: what it reaches, and what it costs. "Terminal" and
                // "command line tools that follow your proxy settings" said
                // neither. The certificate half was stated nowhere at all.
                //
                // The shared sentence, so this row and the popover's blurb
                // cannot describe one control two ways again. It no longer
                // says "Required by OpenCode": OpenCode's configured providers
                // route through a `baseURL` rewrite and need none of this
                // (`useRouting.ts`, the `opencode-env` doc). The channel is
                // what covers a provider added later, and `OpenCodeEnvDialog`
                // is the place that explains that, at the moment it applies.
                description: SHELL_CHANNEL_COVERAGE,
                toggle: {
                  on: shellProxy.on,
                  // Shared with every app switch: this is `useRouting`'s one
                  // `busy` flag, and `setEnvExport` returns early on it. A
                  // machine-wide `launchctl setenv` round trip is exactly the
                  // click someone makes right after flipping an app, so the
                  // swallowed case is reachable rather than theoretical.
                  busy: shellProxy.busy,
                  onToggle: shellProxy.onToggle,
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
                // No description, for the reason Sign-in method has none. What
                // removing it costs is the confirmation dialog's job to say, and
                // that dialog says it before anything happens.
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
        // One row, and one switch over every native notification the app can
        // fire: blocked and flagged requests from the live security feed
        // (AG-578), plus the two routing ones - an expired session, a quit that
        // could not put a tool back. A previous build split it into three rows
        // because AG-594's acceptance criteria names a switch each. The Figma
        // draws one (`116:29086`) with the description below, and the frame wins
        // over the ticket, so the split is gone and the drawn sentence is back on
        // the row it was written for. It under-describes the routing half, which
        // is the price of one switch and is question 8 in
        // `docs/figma-questions-for-design.md`.
        ...(onToggleNotifications
          ? [
              {
                id: "notifications",
                icon: "bell" as IconName,
                label: "Notifications",
                description: "Alert me when a request is blocked or flagged",
                ...(preferencesUnavailable && onRetryPreferences
                  ? { unavailable: { onRetry: onRetryPreferences } }
                  : {
                      toggle: {
                        on: notifications ?? true,
                        onToggle: onToggleNotifications,
                      },
                    }),
              } as SettingsRow,
            ]
          : []),
        // Undrawn, and kept: the frame has no sound row, but AG-594's acceptance
        // criteria names one and it gates something real - `sound` on every
        // security notification the feed fires. Only those: the routing
        // notifications never carry one, which is why the description says
        // security alerts rather than repeating the label.
        //
        // It takes the same `unavailable` branch as the row above because it
        // comes from the same read. It used to lack one, which was survivable
        // while two rows sat between them and is not now they are adjacent: a
        // failed read drew Retry on one row and a confident "On" directly under
        // it, from a value nobody had read.
        ...(onToggleSecurityNotificationSound
          ? [
              {
                id: "security-sound",
                icon: "bell" as IconName,
                label: "Notification sound",
                description: "Play a sound with security alerts",
                ...(preferencesUnavailable && onRetryPreferences
                  ? { unavailable: { onRetry: onRetryPreferences } }
                  : {
                      toggle: {
                        on: securityNotificationSound ?? true,
                        onToggle: onToggleSecurityNotificationSound,
                      },
                    }),
              } as SettingsRow,
            ]
          : []),
      ],
    },
    // Diagnostics gets its own section, out of About: sharing data is a privacy
    // choice, and the report is the evidence of what would be shared.
    //
    // The file draws two rows and this now renders three. That is a deliberate
    // departure, not a drift: the entry this replaces said sending a report on
    // demand "belongs with the collection work and is not here", which was true
    // while no upload existed. AG-603 built one, so the action needs a door, and
    // the Figma has none to copy because AG-603 was blocked on AG-602's handoff
    // and the frames were never drawn. Raised in
    // `docs/figma-questions-for-design.md`.
    //
    // The read-only field list stays a link on the share-diagnostics
    // description rather than becoming a fourth row - it changes no setting and
    // sends nothing, so it is part of that sentence, not an action beside it.
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
                ...(onViewCollectedData
                  ? {
                      descriptionLink: {
                        label: "See what is collected",
                        onClick: onViewCollectedData,
                      },
                    }
                  : {}),
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
        {
          id: "diagnostics-report",
          icon: "clipboardList",
          label: "Diagnostics report",
          description: "Everything Gate knows about this install, as shareable text.",
          action: { label: "View report", onClick: onViewDiagnostics },
        },
        // Below the report, because the report is what this sends: a user who
        // wants to know what they are about to hand over reads it in the row
        // above, and the order on screen matches that.
        ...(onSendDiagnostics
          ? [
              {
                id: "send-diagnostics",
                // `headset`, the support glyph. This row exists to feed a
                // support request, and `share2` is already the switch above.
                icon: "headset" as IconName,
                label: "Send diagnostics now",
                description:
                  "Send one report to Constellation Gate and get a reference for your support request.",
                action: { label: "Send", onClick: onSendDiagnostics },
              } as SettingsRow,
            ]
          : []),
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
    // `relative` for the same reason as `Overview`'s root: this is the scroll
    // container, so anything `sr-only` inside it must take it as containing
    // block rather than hanging below the window as the shell root's hidden
    // overflow. Nothing in Settings is `sr-only` today; the first one would
    // otherwise recreate the Overview's white-space bug here.
    <div className="relative flex flex-1 flex-col gap-6 overflow-auto bg-base-background p-6">
      {/* `heading/20` is 20/24 in the file. The token export's 28 is what
        * `tailwind.config.ts` records, and `text-xl` carries it by default, so
        * the leading is pinned here rather than left to the default. */}
      <h1 className="text-xl font-medium leading-6 tracking-heading text-base-foreground">
        Settings
      </h1>

      {sections.map((section) => (
        <section key={section.id} className="flex flex-col gap-3">
          <h2
            className={`text-base font-medium leading-6 tracking-heading-16 ${
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

      {/* Two row shapes, which is all the design draws. A *value* row -
        * Device, Install ID, Gate plan, Version - sets its label in a fixed
        * 189px column so the values line up down the card, which puts the value
        * at x=233 as every drawn value row does (`116:28991` and siblings). A
        * *description* row
        * - Launch at login, Notifications, Diagnostics report, Reset - gives the
        * text the full width and puts its control at the end.
        *
        * A row carrying both is ours, not the file's: Sign-in method and Gate
        * certificate. They take the description shape, because that column is a
        * gutter and a sentence in it wrapped to six lines. */}
      <div
        className={`min-w-0 ${
          row.description !== undefined ||
          (row.value === undefined && !row.unavailable && !row.valuePending)
            ? "flex-1"
            : "w-[189px] shrink-0"
        }`}
      >
        <p className="truncate text-sm font-medium leading-5 text-base-foreground">
          {row.label}
        </p>
        {row.description && (
          <p className="text-base-xs leading-4 text-base-muted-foreground">
            {row.description}
            {row.descriptionLink && (
              <>
                {" "}
                <button
                  type="button"
                  onClick={row.descriptionLink.onClick}
                  className="rounded-control font-medium text-base-primary underline-offset-2 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
                >
                  {row.descriptionLink.label}
                </button>
              </>
            )}
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
          {row.value === undefined && row.valuePending && (
            <span className="min-w-0 flex-1">
              <Skeleton className="h-4 w-12" />
            </span>
          )}

          {row.value !== undefined && (
            <p
              className={`truncate text-sm leading-5 text-base-foreground ${
                // In a description row the text has taken the width, so the
                // value sits with the control rather than claiming a column.
                //
                // Capped and shrinkable, not `shrink-0`: `truncate` cannot fire
                // on a flex item that refuses to shrink and has no width to
                // truncate against, so the item just grew to its content. A
                // valid 64-character device name - the backend truncates at 128
                // bytes, so it is well inside what a rename accepts - then took
                // the row's whole width, squeezed the description into a
                // one-word column and pushed "Rename device" past the right
                // edge of a 1024px window. `max-w-xs` gives `truncate` a bound
                // to work against and leaves the trailing control its place.
                row.description !== undefined
                  ? "min-w-0 max-w-xs shrink"
                  : "min-w-0 flex-1"
              }`}
              // The full value is still reachable, since the visible text is
              // now an ellipsis of it.
              title={row.value}
            >
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
                busy={row.toggle.busy}
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
