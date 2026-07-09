/**
 * A small inline-SVG icon set. Icons inherit `currentColor` and take an optional
 * pixel size, so they theme with the surrounding text. Keeping them inline avoids
 * any external asset/font dependency.
 */
import type { JSX } from "react";

export type IconName =
  | "lock"
  | "calendar"
  | "sparkle"
  | "github"
  | "download"
  | "terminal"
  | "shield"
  | "wifiOff"
  | "check"
  | "arrowRight"
  | "windows"
  | "apple"
  | "linux"
  | "android"
  | "fingerprint";

interface IconProps {
  name: IconName;
  size?: number;
  className?: string;
}

const paths: Record<IconName, JSX.Element> = {
  lock: (
    <>
      <rect x="4.5" y="10.5" width="15" height="10" rx="2" />
      <path d="M8 10.5V7a4 4 0 0 1 8 0v3.5" />
    </>
  ),
  calendar: (
    <>
      <rect x="3.5" y="5" width="17" height="16" rx="2" />
      <path d="M3.5 9.5h17M8 3v4M16 3v4" />
    </>
  ),
  sparkle: (
    <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3zM18.5 15l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9.9-2.6z" />
  ),
  github: (
    <path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.53 2.34 1.09 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.56 9.56 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2z" />
  ),
  download: (
    <>
      <path d="M12 3v12" />
      <path d="M7 11l5 5 5-5" />
      <path d="M4 20h16" />
    </>
  ),
  terminal: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <path d="M7.5 9.5l3 2.5-3 2.5M12.5 15h4" />
    </>
  ),
  shield: <path d="M12 3l7 3v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6l7-3z" />,
  wifiOff: (
    <>
      <path d="M3 4l18 18" />
      <path d="M8.5 12.5a5 5 0 0 1 7 0M5 9a10 10 0 0 1 4-2.3M19 9a10 10 0 0 0-4.5-2.6" />
      <path d="M11 16a1.5 1.5 0 0 1 2 0" />
    </>
  ),
  check: <path d="M4 12.5l5 5 11-11" />,
  arrowRight: <path d="M4 12h16M14 6l6 6-6 6" />,
  windows: (
    <path d="M3 5.5l7.5-1v7H3v-6zM11.5 4.4L21 3v8.5h-9.5v-7.1zM3 13.5h7.5v6L3 18.5v-5zM11.5 13.5H21V21l-9.5-1.3v-6.2z" />
  ),
  apple: (
    <path d="M16.2 12.9c0-2.3 1.9-3.4 2-3.5-1.1-1.6-2.8-1.8-3.4-1.8-1.4-.1-2.8.9-3.5.9-.7 0-1.9-.8-3.1-.8-1.6 0-3 .9-3.8 2.4-1.6 2.8-.4 7 1.2 9.3.8 1.1 1.7 2.4 2.9 2.3 1.2-.05 1.6-.75 3-.75s1.8.75 3 .73c1.2-.02 2-1.1 2.8-2.2.9-1.3 1.2-2.5 1.3-2.6-.03-.01-2.4-.92-2.4-3.7zM14 6.3c.6-.8 1.1-1.9 1-3-.9.04-2.1.6-2.8 1.4-.6.7-1.1 1.8-1 2.9 1 .08 2.1-.5 2.8-1.3z" />
  ),
  linux: (
    <path d="M12 2c-2 0-3 1.8-3 4 0 1.4.2 2.2-.6 3.4C7.2 11.2 6 12.6 6 14.6c0 .8.3 1.3-.2 2.2-.4.7-1.3 1.2-1.3 2 0 .7.7 1 1.7 1.2 1.2.3 2 .8 3 .8s1.4-.6 2.8-.6 1.8.6 2.8.6 1.8-.5 3-.8c1-.2 1.7-.5 1.7-1.2 0-.8-.9-1.3-1.3-2-.5-.9-.2-1.4-.2-2.2 0-2-1.2-3.4-2.4-5.2-.8-1.2-.6-2-.6-3.4 0-2.2-1-4-3-4zm-1.6 6.1c.5 0 .9.5.9 1s-.4.8-.9.8-.9-.3-.9-.8.4-1 .9-1zm3.2 0c.5 0 .9.5.9 1s-.4.8-.9.8-.9-.3-.9-.8.4-1 .9-1z" />
  ),
  android: (
    <path d="M8 8a4 4 0 0 1 8 0zM7 9.2h10V17a1 1 0 0 1-1 1h-1.2v2.8a1 1 0 0 1-2 0V18h-1.6v2.8a1 1 0 0 1-2 0V18H8a1 1 0 0 1-1-1zM4.8 9.6a1 1 0 0 1 1 1v4.8a1 1 0 0 1-2 0v-4.8a1 1 0 0 1 1-1zm14.4 0a1 1 0 0 1 1 1v4.8a1 1 0 0 1-2 0v-4.8a1 1 0 0 1 1-1z" />
  ),
  fingerprint: (
    <>
      <path d="M4 12a8 8 0 0 1 16 0" />
      <path d="M7 12a5 5 0 0 1 10 0v2" />
      <path d="M9.5 12a2.5 2.5 0 0 1 5 0v3.5" />
      <path d="M12 12v5.5" />
      <path d="M7 15v2M17 14.5v2.5" />
    </>
  ),
};

/** Every valid icon name, for runtime validation of content data. */
export const iconNames = Object.keys(paths) as IconName[];

// Icons that read best as solid fills rather than strokes.
const filled = new Set<IconName>([
  "sparkle",
  "github",
  "shield",
  "windows",
  "apple",
  "linux",
  "android",
]);

export function Icon({ name, size = 24, className }: IconProps) {
  const solid = filled.has(name);
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={solid ? "currentColor" : "none"}
      stroke={solid ? "none" : "currentColor"}
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {paths[name]}
    </svg>
  );
}
