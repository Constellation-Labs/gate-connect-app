import { useId } from "react";

/**
 * Constellation faceted hex mark — exact port of the Claude Design prototype's
 * `ConstellationHexMark` (brand-lockup.jsx). Gradients + path geometry are
 * reproduced verbatim so the logo is pixel-identical to the handoff.
 */

const NAVY = "#002a5f";
const ACCENT = "#3e4fea";

type GradSpec = { x1: number; y1: number; x2: number; y2: number; stops: [string, number][] };

const PATHS: { d: string; grad?: string; solid?: boolean; ord?: number }[] = [
  { d: "M25.85,22.41V7.47l-4.31,7.47Z", grad: "a" },
  { d: "M12.93,14.94,8.62,17.42v-5Z", grad: "b" },
  { d: "M21.54,14.94l-4.3-7.47h8.61Z", grad: "c" },
  { d: "M21.54 14.94 L12.93 29.88 L25.85 22.41 Z", grad: "d" },
  { d: "M0 7.47 L0 22.41 L8.62 22.41 Z", grad: "e" },
  { d: "M0 7.47 L12.93 0 L25.85 7.47 L8.62 7.47 L4.31 14.94 Z", solid: true, ord: 5 },
  {
    d: "M8.62 17.42 L12.93 19.91 L17.24 17.42 L17.24 12.45 L12.93 9.97 L8.62 12.45 L12.93 14.94 Z",
    solid: true,
    ord: 6,
  },
  { d: "M0 22.41 L17.24 22.41 L12.93 29.88 Z", solid: true, ord: 7 },
];

const GRADS: Record<string, GradSpec> = {
  a: { x1: 20.47, y1: 5.61, x2: 29.09, y2: 20.54, stops: [["0", 0.25], ["1", 1]] },
  b: { x1: 8.51, y1: 12.51, x2: 10.67, y2: 16.01, stops: [["0", 0.25], ["1", 1]] },
  c: { x1: 20.47, y1: 5.61, x2: 29.09, y2: 20.55, stops: [["0", 1], ["1", 0.25]] },
  d: { x1: 7.54, y1: 46.68, x2: 24.78, y2: 16.8, stops: [["0.47", 1], ["1", 0.25]] },
  e: { x1: 4.31, y1: 22.41, x2: 4.31, y2: 7.47, stops: [["0", 1], ["1", 0.25]] },
};

export function ConstellationHexMark({
  size = 22,
  fill = NAVY,
  highlight,
  title,
}: {
  size?: number;
  fill?: string;
  highlight?: "inner";
  title?: string;
}) {
  const uid = useId();
  const g = (k: string) => `${uid}-${k}`;
  return (
    <svg
      width={size}
      height={size * (29.88 / 25.85)}
      viewBox="0 0 25.85 29.88"
      aria-label={title}
      style={{ flexShrink: 0 }}
    >
      <defs>
        {Object.entries(GRADS).map(([k, s]) => (
          <linearGradient
            key={k}
            id={g(k)}
            x1={s.x1}
            y1={s.y1}
            x2={s.x2}
            y2={s.y2}
            gradientUnits="userSpaceOnUse"
          >
            {s.stops.map(([off, op], i) => (
              <stop key={i} offset={off} stopColor={fill} stopOpacity={op} />
            ))}
          </linearGradient>
        ))}
      </defs>
      {PATHS.map((p, i) => (
        <path
          key={i}
          d={p.d}
          fill={
            p.solid
              ? highlight === "inner" && p.ord === 6
                ? ACCENT
                : fill
              : `url(#${g(p.grad!)})`
          }
        />
      ))}
    </svg>
  );
}
