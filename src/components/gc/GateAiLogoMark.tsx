import { useId } from "react";

/**
 * The Gate AI faceted hex mark, verbatim from the tray flow's header
 * (`gate-ai-logo-mark 1`, Figma `Flows / Tray` 694:34009, exported
 * 2026-08-28). Distinct from `ConstellationHexMark`: this is the Gate AI
 * brand mark the tray lockup draws, not the app's own hex.
 *
 * Gradient ids are scoped with `useId` so two instances on one page cannot
 * capture each other's defs - same hazard `ConstellationHexMark` guards.
 *
 * The drawn box is 23.33 x 27.16; `height` scales it with the aspect kept.
 */
export function GateAiLogoMark({ height = 27 }: { height?: number }) {
  const uid = useId();
  const g = (name: string) => `${uid}-${name}`;
  return (
    <svg
      aria-hidden
      width={(height * 23.3333) / 27.1642}
      height={height}
      viewBox="0 0 23.3333 27.1642"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M22.9767 19.9653V6.88729L19.2039 13.4263L22.9767 19.9653Z" fill={`url(#${g("a")})`} />
      <path d="M19.2038 13.4263L15.4397 6.88729H22.9767L19.2038 13.4263Z" fill={`url(#${g("b")})`} />
      <path d="M19.2039 13.4263L11.667 26.5043L22.9768 19.9653L19.2039 13.4263Z" fill={`url(#${g("c")})`} />
      <path d="M0.348389 6.88729V19.9653H7.89407L0.348389 6.88729Z" fill={`url(#${g("d")})`} />
      <path d="M0.348389 6.88723L11.6669 0.348216L22.9767 6.88723H7.89407L4.12125 13.4263L0.348389 6.88723Z" fill="#142666" />
      <path d="M0.348389 19.9654H15.4398L11.6669 26.5044L0.348389 19.9654Z" fill="#142666" />
      <path d="M11.7119 13.4808V17.8822L7.89398 15.678V11.2765L11.7119 13.4808Z" fill={`url(#${g("e")})`} />
      <path d="M11.7119 17.8787L15.5298 15.6745V11.273L11.7119 9.07579L7.89398 11.273L11.7119 13.4773V17.8787Z" fill="#142666" />
      <path opacity="0.2" d="M15.5297 11.2483V15.6779L11.7119 17.8822V13.4807L15.5297 11.2483Z" fill={`url(#${g("f")})`} />
      <defs>
        <linearGradient id={g("a")} x1="18.2672" y1="5.25907" x2="25.8129" y2="18.3284" gradientUnits="userSpaceOnUse">
          <stop stopColor="#1F316E" />
          <stop offset="1" stopColor="#6581C7" />
        </linearGradient>
        <linearGradient id={g("b")} x1="18.2672" y1="5.25907" x2="25.8128" y2="18.3371" gradientUnits="userSpaceOnUse">
          <stop stopColor="#7388C9" />
          <stop offset="1" stopColor="#1C2E6B" />
        </linearGradient>
        <linearGradient id={g("c")} x1="6.94875" y1="41.2105" x2="22.0401" y2="15.0545" gradientUnits="userSpaceOnUse">
          <stop offset="0.47" stopColor="#22316C" />
          <stop offset="1" stopColor="#90AAEF" />
        </linearGradient>
        <linearGradient id={g("d")} x1="4.12125" y1="6.88729" x2="4.12125" y2="19.9653" gradientUnits="userSpaceOnUse">
          <stop stopColor="#A2B7F9" />
          <stop offset="1" stopColor="#142666" />
        </linearGradient>
        <linearGradient id={g("e")} x1="7.81434" y1="11.3618" x2="11.5522" y2="17.4194" gradientUnits="userSpaceOnUse">
          <stop stopColor="#96B1F8" />
          <stop offset="0.12" stopColor="#8FAAF0" />
          <stop offset="0.31" stopColor="#7E97DC" />
          <stop offset="0.53" stopColor="#6178BB" />
          <stop offset="0.79" stopColor="#394D8D" />
          <stop offset="1" stopColor="#132461" />
        </linearGradient>
        <linearGradient id={g("f")} x1="15.6155" y1="11.3442" x2="11.8689" y2="17.4193" gradientUnits="userSpaceOnUse">
          <stop stopColor="#96B1F8" />
          <stop offset="1" stopColor="#132461" />
        </linearGradient>
      </defs>
    </svg>
  );
}
