# Figma dialog frames vs `dialogs.tsx` / `Modal.tsx`

Audit date 2026-09-03. Read-only on source; findings only.

Method: `get_metadata` per page (dumped to scratch XML and measured with
scripts, never eyeballed), `get_screenshot` per dialog frame,
`get_variable_defs` on the specific node before calling any colour wrong.
Every number below marked MEASURED came off a frame's own
`width`/`height`/variable set; anything marked INFERRED is reasoning from
the design contract, not a frame.

Code under audit: `src/components/gc/Modal.tsx` (671 lines),
`src/components/gc/dialogs.tsx` (1736 lines).

## Frame inventory (measured)

### Flows / Overview (`116:26381`)

| screen | dialog node | w x h |
| --- | --- | --- |
| overview-switch org | `130:55314` | 512 x 380 |
| overview-switch org (newer) | `143:68237` | **520** x 356 |
| overview-switch org (switched) | `130:55755` | 512 x 244 |
| overview-review config | `130:57442` | 600 x 418 |
| overview-apply changes | `130:58427`, `135:62184` | 600 x 318 |
| overview-close apps | `130:58855`, `135:62601` | 600 x 318 |
| overview-change ready | `134:61659`, `135:63018` | 512 x 244 |
| overview-quit (chooser) | `694:32272` | 600 x 428 |
| overview-quit (confirm A) | `694:33002` | **536** x 232 |
| overview-quit (confirm B) | `694:33340` | **536** x 232 |

