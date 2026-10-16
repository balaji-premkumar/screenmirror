import { FC } from 'react';

/** A phone and a screen joined by a cable: the whole product in a mark. */
const LogoIcon: FC<{ size?: number }> = ({ size = 28 }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect x="2.5" y="7" width="8" height="18" rx="2.5" className="stroke-ink" strokeWidth="2" />
    <rect x="19.5" y="9" width="10" height="14" rx="2.5" className="stroke-ink" strokeWidth="2" />
    <path d="M10.5 18c3 0 4-5 9-5" className="stroke-signal" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

export default LogoIcon;
